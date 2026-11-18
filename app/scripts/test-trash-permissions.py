#!/usr/bin/env python3
"""Exercise the production macOS deletion path without touching personal Trash.

Unit tests use test-trash, so run this against a built stology-core executable.
The sandbox permits writes into Trash but denies opening the directory, matching
the permission boundary that previously failed after the file had already moved.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    with tempfile.TemporaryDirectory(prefix="stology-trash-permissions-") as temp:
        home = Path(temp).resolve()  # sandbox paths must use /private/var on macOS
        root = home / "data"
        trash = home / ".Trash"
        trash.mkdir()
        env = dict(os.environ, HOME=str(home))
        deny_read = f'(version 1)(allow default)(deny file-read-data (literal "{trash}"))'
        deny_write = f'(version 1)(allow default)(deny file-write* (subpath "{trash}"))'

        def call(op, *, profile=None, fail=False, **kw):
            cmd = [binary, "--root", str(root), "--request"]
            if profile:
                cmd = ["/usr/bin/sandbox-exec", "-p", profile] + cmd
            proc = subprocess.run(
                cmd, input=json.dumps(dict(op=op, **kw)), text=True,
                capture_output=True, env=env, check=False,
            )
            value = json.loads(proc.stdout)
            assert value["ok"] is not fail, value
            assert proc.returncode == (1 if fail else 0), proc.stderr
            return value if fail else value["result"]

        workspace = call("workspace.create", name="휴지통 권한 회귀 테스트")["id"]
        for kind in ("note", "draft", "material"):
            body = f"# 복구 가능한 {kind}\n"
            draft = call("draft.create", workspace_id=workspace, body=body)
            item = draft if kind == "draft" else call(
                "note.save", draft_id=draft["id"], title=f"권한 테스트 {kind}"
            )
            # Actual write failure must retain both the original and its DB row.
            call(f"{kind}.delete", id=item["id"], profile=deny_write, fail=True)
            read_op = "draft.get" if kind == "draft" else "note.read"
            assert call(read_op, id=item["id"])["body"] == body
            assert not list(trash.iterdir())

            # Directory-read denial must not turn a completed move into failure.
            deleted = call(f"{kind}.delete", id=item["id"], profile=deny_read)
            destination = Path(deleted["trash_path"])
            assert destination.parent == trash
            assert destination.read_text() == body
            if kind != "draft":
                assert not Path(item["absolute_path"]).exists()
            call(read_op, id=item["id"], profile=deny_read, fail=True)
            call(
                "draft.update" if kind == "draft" else "note.update",
                id=item["id"], body="late save", profile=deny_read, fail=True,
            )
            assert not list((root / "pending-deletions").iterdir())
            destination.unlink()  # Only our isolated fixture, never personal Trash.
            print(f"PASS {kind}: protected Trash / write failure / restart / late save")

        # Reproduce an old build's post-move error: a journal and moved file with
        # an uncommitted tombstone must be recovered on the next request.
        draft = call("draft.create", workspace_id=workspace, body="old interrupted delete")
        item = call("note.save", draft_id=draft["id"], title="이전 삭제 복구")
        destination = trash / "recovery.md"
        intent = dict(id=item["id"], source=item["absolute_path"],
                      destination=str(destination), body=None)
        journal = root / "pending-deletions" / f'{item["id"]}.json'
        journal.write_text(json.dumps(intent))
        Path(item["absolute_path"]).rename(destination)
        assert not call("tree", workspace_id=workspace, profile=deny_read)["entries"]
        assert destination.read_text() == "old interrupted delete"
        assert not journal.exists()
        print("PASS recovery: previously moved file reconciled with database")


if __name__ == "__main__":
    main()
