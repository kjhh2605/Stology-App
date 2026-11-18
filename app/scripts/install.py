#!/usr/bin/env python3
"""Install the signed build at the canonical Dock path, without keeping an archived copy."""
from pathlib import Path
import ctypes
import os
import plistlib
import shutil
import subprocess
import tempfile
import sys


DESTINATION = Path("/Applications/Stology Local.app")


def executable_path(pid):
    """Read the kernel's executable path, independent of arguments or app name."""
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
    libproc.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    libproc.proc_pidpath.restype = ctypes.c_int
    buffer = ctypes.create_string_buffer(4096)
    if libproc.proc_pidpath(pid, buffer, len(buffer)) <= 0:
        return None
    return Path(os.fsdecode(buffer.value))


def destination_running():
    processes = subprocess.run(["pgrep", "-x", "stology-local"], capture_output=True, text=True)
    if processes.returncode == 1:
        return False
    if processes.returncode != 0:
        raise RuntimeError("설치 대상 앱의 실행 상태를 확인하지 못했습니다.")
    for pid in processes.stdout.split():
        executable = executable_path(int(pid))
        # Unknown paths remain blocking. A separate demo/QA bundle is not replaced.
        if executable is None or executable.resolve().is_relative_to(DESTINATION.resolve()):
            return True
    return False


def require_destination_idle():
    if destination_running():
        raise RuntimeError("Stology Local을 종료한 뒤 설치하세요. 실행 중인 편집 내용을 자동으로 종료하지 않습니다.")


def install():
    root = Path(__file__).resolve().parent.parent
    source = root / "dist/Stology Local.app"
    destination = DESTINATION
    require_destination_idle()
    info = plistlib.loads((source / "Contents/Info.plist").read_bytes())
    if info.get("CFBundleIdentifier") != "app.stology.local":
        raise RuntimeError("정식 로컬앱 빌드가 아닙니다. QA 앱은 설치하지 않습니다.")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(source)], check=True)
    if destination.is_symlink():
        raise RuntimeError("설치 대상이 심볼릭 링크입니다. 기존 경로를 보존했습니다.")
    staging = Path(tempfile.mkdtemp(prefix=".stology-install-", dir="/Applications"))
    incoming = staging / "Stology Local.app"
    previous = staging / "previous.app"
    try:
        subprocess.run(["ditto", str(source), str(incoming)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(incoming)], check=True)
        # 이전 설치본은 ZIP으로 따로 보관하지 않는다. 교체 실패 시 복구용 임시 사본만 둔다.
        # Preparing the staged copy/archive can take time. Do not replace an app
        # the user opened while that work was running.
        if destination_running():
            raise RuntimeError("준비 중 앱이 실행되어 설치를 중단했습니다. 앱 종료 후 다시 설치하세요.")
        if destination.exists():
            os.replace(destination, previous)
        try:
            os.replace(incoming, destination)
        except BaseException:
            if previous.exists():
                os.replace(previous, destination)
            raise
        print(f"설치 완료: {destination}\nDock 또는 Applications에서 이 앱을 실행하세요.")
    finally:
        # Never remove the rollback if replacement/restoration itself failed.
        if previous.exists() and not destination.exists():
            print(f"이전 앱 복구 위치: {previous}")
        else:
            shutil.rmtree(staging)


if __name__ == "__main__":
    if sys.argv[1:] == ["--check-idle"]:
        require_destination_idle()
    else:
        install()
