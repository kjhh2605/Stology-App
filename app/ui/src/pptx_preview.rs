//! Optional, non-bundled LibreOffice fallback for systems whose Quick Look only
//! displays a PPTX icon. Work is off the AppKit thread; originals never change.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const std::ffi::c_void,
        buffer: *const u8,
        length: isize,
        is_directory: u8,
    ) -> *const std::ffi::c_void;
    fn CFRelease(value: *const std::ffi::c_void);
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPDFDocumentCreateWithURL(url: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGPDFDocumentGetNumberOfPages(document: *mut std::ffi::c_void) -> usize;
    fn CGPDFDocumentIsUnlocked(document: *mut std::ffi::c_void) -> bool;
    fn CGPDFDocumentRelease(document: *mut std::ffi::c_void);
}

// Bump when the renderer environment changes; previews made with older font
// discovery rules must never be reused merely because the PPTX bytes match.
const RENDERER_CONFIG_VERSION: &str = "libreoffice-pdf-v2-private-macos-fontconfig";
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SOURCE: u64 = 128 * 1024 * 1024;
const MAX_PDF: u64 = 64 * 1024 * 1024;
const MAX_CACHE: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 16;
type ConversionResult = Result<String, String>;

#[derive(Default)]
struct Shared {
    cancelled: AtomicBool,
    // Own the Child behind the lock so a cancellation never signals a PID that
    // has already been reaped (and could have been recycled by the OS).
    child: Mutex<Option<Child>>,
}
pub struct Handle {
    shared: Arc<Shared>,
    receiver: Mutex<mpsc::Receiver<ConversionResult>>,
    delivered: AtomicBool,
    worker: Option<JoinHandle<()>>,
}
impl Handle {
    /// None while pending (and after the single result has been consumed).
    pub fn try_result(&self) -> Option<ConversionResult> {
        if self.delivered.load(Ordering::Acquire) {
            return None;
        }
        let result = match self.receiver.lock().unwrap().try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("PPTX 미리보기 변환 작업이 종료되었습니다.".into()))
            }
        };
        if result.is_some() {
            self.delivered.store(true, Ordering::Release);
        }
        result
    }
    /// Signals the entire process group immediately. The worker reaps the child;
    /// Drop also joins it, so app termination cannot orphan the converter.
    pub fn cancel(&self) {
        self.shared.cancelled.store(true, Ordering::Release);
        if let Some(child) = self.shared.child.lock().unwrap().as_mut() {
            terminate_group(child);
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn available() -> bool {
    executable().is_some()
}
pub fn start(path: &str) -> Handle {
    let shared = Arc::new(Shared::default());
    let (sender, receiver) = mpsc::channel();
    let state = shared.clone();
    let path = PathBuf::from(path);
    let worker = thread::spawn(move || {
        let result = convert(&path, &state);
        // The view may have gone away between conversion and result delivery.
        let result = if state.cancelled.load(Ordering::Acquire) {
            Err("PPTX 미리보기를 취소했습니다.".into())
        } else {
            result
        };
        let _ = sender.send(result);
    });
    Handle {
        shared,
        receiver: Mutex::new(receiver),
        delivered: AtomicBool::new(false),
        worker: Some(worker),
    }
}
fn executable() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
    )];
    if let Some(home) = std::env::var_os("HOME") {
        candidates
            .push(PathBuf::from(home).join("Applications/LibreOffice.app/Contents/MacOS/soffice"));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/soffice"),
        PathBuf::from("/usr/local/bin/soffice"),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("soffice")),
        );
    }
    candidates
        .into_iter()
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}
fn cache_directory() -> Result<PathBuf, String> {
    let root = if let Some(root) = std::env::var_os("STOLOGY_DATA_DIR") {
        PathBuf::from(root)
    } else {
        PathBuf::from(std::env::var_os("HOME").ok_or("사용자 홈 디렉터리를 찾을 수 없습니다.")?)
            .join("Library/Application Support/Stology Local")
    };
    let path = root.join("preview-cache");
    fs::create_dir_all(&path).map_err(|e| format!("미리보기 캐시를 만들지 못했습니다: {e}"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("미리보기 캐시 권한 오류: {e}"))?;
    Ok(path)
}
fn check_cancel(shared: &Shared, started: Instant) -> Result<(), String> {
    if shared.cancelled.load(Ordering::Acquire) {
        Err("PPTX 미리보기를 취소했습니다.".into())
    } else if started.elapsed() >= TIMEOUT {
        Err("PPTX 변환이 60초 안에 끝나지 않았습니다. PDF로 내보낸 자료를 열어 주세요.".into())
    } else {
        Ok(())
    }
}
fn terminate_group(child: &mut Child) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
}
fn file_url(path: &Path) -> String {
    // Percent-encode bytes rather than passing a raw space/#/% in LO's URL.
    let mut url = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            url.push(*byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}
fn valid_pdf(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if metadata.len() < 32 || metadata.len() > MAX_PDF {
        return false;
    }
    let mut magic = [0; 5];
    if file.read_exact(&mut magic).is_err() || &magic != b"%PDF-" {
        return false;
    }
    let tail_size = metadata.len().min(4096);
    if file.seek(SeekFrom::End(-(tail_size as i64))).is_err() {
        return false;
    }
    let mut tail = Vec::with_capacity(tail_size as usize);
    if file.read_to_end(&mut tail).is_err() {
        return false;
    }
    let trimmed = tail.trim_ascii_end();
    if !trimmed.ends_with(b"%%EOF") {
        return false;
    }
    let Some(offset) = trimmed.windows(9).rposition(|w| w == b"startxref") else {
        return false;
    };
    let xref = String::from_utf8_lossy(&trimmed[offset + 9..]);
    let sane_xref = xref
        .split_whitespace()
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .is_some_and(|n| n >= 8 && n < metadata.len());
    sane_xref && native_pdf_has_pages(path)
}
/// CoreGraphics performs actual PDF parsing on this worker thread. Signature
/// and trailer checks alone cannot distinguish a corrupt cached file from PDF.
fn native_pdf_has_pages(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        );
        if url.is_null() {
            return false;
        }
        let document = CGPDFDocumentCreateWithURL(url);
        CFRelease(url);
        if document.is_null() {
            return false;
        }
        let valid =
            CGPDFDocumentIsUnlocked(document) && CGPDFDocumentGetNumberOfPages(document) > 0;
        CGPDFDocumentRelease(document);
        valid
    }
}

/// Leave an exited group leader unreaped until its descendants have been
/// terminated. This reserves the group id and avoids signalling a recycled PID.
fn exited_without_reaping(child: &Child) -> std::io::Result<bool> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(info.si_pid != 0)
    }
}
fn finish_exited_group(child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
    terminate_group(child);
    child.wait()
}
fn lock_file(path: &Path, shared: Option<(&Shared, Instant)>) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("미리보기 캐시 잠금 오류: {e}"))?;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(format!("미리보기 캐시 잠금 오류: {error}"));
        }
        if let Some((state, start)) = shared {
            check_cancel(state, start)?;
        }
        thread::sleep(Duration::from_millis(20));
    }
}
struct ConversionDirectory {
    // Drop the directory while the lease remains held, so another converter
    // cannot scavenge it during removal. Rust drops fields in declaration order.
    directory: tempfile::TempDir,
    _lease: File,
}
impl ConversionDirectory {
    fn path(&self) -> &Path {
        self.directory.path()
    }
}
fn conversion_directory(
    cache: &Path,
    shared: &Shared,
    started: Instant,
) -> Result<ConversionDirectory, String> {
    let _registry = lock_file(&cache.join(".conversions.lock"), Some((shared, started)))?;
    scavenge_conversions(cache);
    let directory = tempfile::Builder::new()
        .prefix("conversion-")
        .tempdir_in(cache)
        .map_err(|e| format!("변환 임시 디렉터리 오류: {e}"))?;
    let lease = lock_file(
        &directory.path().join(".active.lock"),
        Some((shared, started)),
    )?;
    Ok(ConversionDirectory {
        directory,
        _lease: lease,
    })
}
/// Caller holds .conversions.lock; active directories are protected by their
/// lease even across multiple app processes. Never follow symlink directories.
fn scavenge_conversions(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("conversion-")
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let directory = entry.path();
        let lease_path = directory.join(".active.lock");
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lease_path);
        match lease {
            Ok(file) => {
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    let _ = fs::remove_dir_all(&directory);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Previous builds did not write leases. Avoid deleting a live
                // legacy conversion, whose own deadline was 60 seconds.
                if entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age >= TIMEOUT * 2)
                {
                    let _ = fs::remove_dir_all(&directory);
                }
            }
            Err(_) => {}
        }
    }
}
fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}
fn font_directories() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join("Library/Fonts"));
    }
    paths.into_iter().filter(|p| p.is_dir()).collect()
}
fn font_configuration(directories: &[PathBuf], cache: &Path) -> String {
    let mut xml=String::from("<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\"><fontconfig>");
    for directory in directories {
        xml.push_str(&format!(
            "<dir>{}</dir>",
            xml_text(&directory.to_string_lossy())
        ));
    }
    xml.push_str(&format!("<cachedir>{}</cachedir><alias><family>sans-serif</family><prefer><family>Arial Unicode MS</family></prefer></alias></fontconfig>",xml_text(&cache.to_string_lossy())));
    xml
}
fn convert(source: &Path, shared: &Shared) -> ConversionResult {
    let started = Instant::now();
    check_cancel(shared, started)?;
    if !source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pptx"))
    {
        return Err("PPTX 파일만 원본 미리보기로 변환할 수 있습니다.".into());
    }
    let mut input = File::open(source).map_err(|e| format!("PPTX 원본을 읽지 못했습니다: {e}"))?;
    let metadata = input
        .metadata()
        .map_err(|e| format!("PPTX 파일 정보 오류: {e}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("비어 있거나 일반 파일이 아닌 PPTX입니다.".into());
    }
    if metadata.len() > MAX_SOURCE {
        return Err("128MB를 넘는 PPTX는 PDF로 내보낸 뒤 열어 주세요.".into());
    }
    let cache = cache_directory()?;
    prune_cache(&cache, None);
    let temporary = conversion_directory(&cache, shared, started)?;
    let snapshot = temporary.path().join("input.pptx");
    let mut copy = File::create(&snapshot).map_err(|e| format!("PPTX 입력 사본 오류: {e}"))?;
    let mut digest = Sha256::new();
    digest.update(RENDERER_CONFIG_VERSION.as_bytes());
    digest.update([0]);
    let font_directories = font_directories();
    for directory in &font_directories {
        digest.update(directory.as_os_str().as_encoded_bytes());
        digest.update([0]);
    }
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        check_cancel(shared, started)?;
        let read = input
            .read(&mut buffer)
            .map_err(|e| format!("PPTX 원본 읽기 오류: {e}"))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > MAX_SOURCE {
            return Err("변환 중 파일이 128MB를 넘었습니다.".into());
        }
        digest.update(&buffer[..read]);
        copy.write_all(&buffer[..read])
            .map_err(|e| format!("PPTX 입력 사본 쓰기 오류: {e}"))?;
    }
    copy.sync_all()
        .map_err(|e| format!("PPTX 입력 사본 동기화 오류: {e}"))?;
    drop(copy);
    let hash = format!("{:x}", digest.finalize());
    let name: String = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("slides")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    let target = cache.join(format!(
        "pptx-{hash}-{}.pdf",
        if name.is_empty() { "slides" } else { &name }
    ));
    {
        let _publication = lock_file(&cache.join(".publication.lock"), Some((shared, started)))?;
        if valid_pdf(&target) {
            check_cancel(shared, started)?;
            return Ok(target.to_string_lossy().into_owned());
        }
        if target.exists() {
            fs::remove_file(&target)
                .map_err(|e| format!("손상된 미리보기 캐시를 정리하지 못했습니다: {e}"))?;
        }
    }
    let binary=executable().ok_or("LibreOffice가 설치되어 있지 않습니다. PPTX를 PDF로 내보내거나 시스템 미리보기를 사용해 주세요.")?;
    let profile = temporary.path().join("profile");
    let user = profile.join("user");
    fs::create_dir_all(&user).map_err(|e| format!("변환 전용 설정 디렉터리 오류: {e}"))?;
    fs::write(user.join("registrymodifications.xcu"),r#"<?xml version="1.0" encoding="UTF-8"?><oor:items xmlns:oor="http://openoffice.org/2001/registry"><item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item><item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="FirstRun" oor:op="fuse"><value>false</value></prop></item></oor:items>"#).map_err(|e|format!("변환 전용 설정 쓰기 오류: {e}"))?;
    // LibreOffice's macOS headless font discovery can omit Korean fallback
    // fonts. This private config references installed fonts without installing,
    // copying or changing any font or the user's application configuration.
    let font_cache = temporary.path().join("font-cache");
    fs::create_dir(&font_cache).map_err(|e| format!("변환 폰트 캐시 디렉터리 오류: {e}"))?;
    let font_config = temporary.path().join("fonts.conf");
    fs::write(
        &font_config,
        font_configuration(&font_directories, &font_cache),
    )
    .map_err(|e| format!("변환 폰트 설정 오류: {e}"))?;
    let output = temporary.path().join("output");
    fs::create_dir(&output).map_err(|e| format!("미리보기 출력 디렉터리 오류: {e}"))?;
    let mut command = Command::new(binary);
    command
        .arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .args([
            "--headless",
            "--nologo",
            "--nodefault",
            "--norestore",
            "--nolockcheck",
            "--convert-to",
            "pdf:impress_pdf_Export",
            "--outdir",
        ])
        .arg(&output)
        .arg(&snapshot)
        .current_dir(temporary.path())
        .env("FONTCONFIG_FILE", &font_config)
        .env_remove("FONTCONFIG_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    {
        let mut slot = shared.child.lock().unwrap();
        check_cancel(shared, started)?;
        *slot = Some(
            command
                .spawn()
                .map_err(|e| format!("LibreOffice를 실행하지 못했습니다: {e}"))?,
        );
    }
    let status = loop {
        let cancellation = check_cancel(shared, started);
        let mut slot = shared.child.lock().unwrap();
        let child = slot.as_mut().expect("converter child");
        if let Err(error) = cancellation {
            terminate_group(child);
            let _ = child.wait();
            slot.take();
            return Err(error);
        }
        match exited_without_reaping(child) {
            Ok(true) => {
                let status = finish_exited_group(child)
                    .map_err(|e| format!("변환 프로세스 회수 오류: {e}"))?;
                slot.take();
                break status;
            }
            Ok(false) => {}
            Err(error) => {
                terminate_group(child);
                let _ = child.wait();
                slot.take();
                return Err(format!("변환 프로세스 확인 오류: {error}"));
            }
        }
        drop(slot);
        thread::sleep(Duration::from_millis(40));
    };
    check_cancel(shared, started)?;
    if !status.success() {
        return Err(format!(
            "LibreOffice가 PPTX를 변환하지 못했습니다 ({status}). 원본을 PDF로 내보내 주세요."
        ));
    }
    let pdf = output.join("input.pdf");
    if !valid_pdf(&pdf) {
        return Err("LibreOffice가 유효한 PDF를 만들지 못했습니다. 손상·암호화·지원되지 않는 PPTX인지 확인하거나 PDF로 내보내 주세요.".into());
    }
    File::open(&pdf)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("변환 PDF 저장 오류: {e}"))?;
    let _publication = lock_file(&cache.join(".publication.lock"), Some((shared, started)))?;
    if target.exists() && !valid_pdf(&target) {
        fs::remove_file(&target)
            .map_err(|e| format!("손상된 미리보기 캐시를 교체하지 못했습니다: {e}"))?;
    }
    match fs::hard_link(&pdf, &target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && valid_pdf(&target) => {}
        Err(error) => return Err(format!("미리보기 PDF 캐시 저장 오류: {error}")),
    }
    prune_cache(&cache, Some(&target));
    check_cancel(shared, started)?;
    Ok(target.to_string_lossy().into_owned())
}
fn prune_cache(cache: &Path, keep: Option<&Path>) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("pptx-") || !name.ends_with(".pdf") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((
                path,
                metadata.len(),
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|f| f.2);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let mut count = files.len();
    for (path, size, _) in files {
        if count <= MAX_FILES && total <= MAX_CACHE {
            break;
        }
        if keep == Some(path.as_path()) {
            continue;
        }
        if fs::remove_file(path).is_ok() {
            count -= 1;
            total = total.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_url_encodes_spaces_and_fragment() {
        assert_eq!(
            file_url(Path::new("/tmp/a b/#c%")),
            "file:///tmp/a%20b/%23c%25"
        );
    }
    #[test]
    fn cancellation_before_spawn_is_observed() {
        let state = Shared::default();
        state.cancelled.store(true, Ordering::Release);
        assert!(check_cancel(&state, Instant::now()).is_err());
    }
    #[test]
    fn cache_prunes_only_managed_pdfs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), b"user").unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("pptx-{i}.pdf")), b"%PDF-1.4\n").unwrap();
        }
        prune_cache(dir.path(), None);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 17);
        assert!(dir.path().join("keep.txt").exists());
    }
    #[test]
    fn interrupted_conversion_directories_are_scavenged_but_active_lease_survives() {
        let cache = tempfile::tempdir().unwrap();
        let shared = Shared::default();
        let active = conversion_directory(cache.path(), &shared, Instant::now()).unwrap();
        fs::write(active.path().join("input.pptx"), b"active source").unwrap();
        let stale = cache.path().join("conversion-crashed");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join(".active.lock"), b"").unwrap();
        fs::write(stale.join("input.pptx"), b"stale source").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("input.pptx"), b"untouched").unwrap();
        std::os::unix::fs::symlink(outside.path(), cache.path().join("conversion-symlink"))
            .unwrap();
        let _registry = lock_file(&cache.path().join(".conversions.lock"), None).unwrap();
        scavenge_conversions(cache.path());
        assert!(active.path().join("input.pptx").exists());
        assert!(!stale.exists());
        assert!(outside.path().join("input.pptx").exists());
        let active_path = active.path().to_path_buf();
        drop(active);
        assert!(!active_path.exists());
    }
    #[test]
    fn pdf_validation_rejects_header_only_and_missing_eof() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("broken.pdf");
        fs::write(&p, b"%PDF-1.7\ntruncated content without trailer or xref").unwrap();
        assert!(!valid_pdf(&p));
        fs::write(
            &p,
            b"%PDF-1.7\n0000000000000000000000000000000000\nstartxref\n9999999\n%%EOF",
        )
        .unwrap();
        assert!(!valid_pdf(&p));
        fs::write(&p, b"%PDF-1.4\njunkjunkjunkjunk\nstartxref\n9\n%%EOF\n").unwrap();
        assert!(
            !valid_pdf(&p),
            "native parser must reject a spoofed header/trailer"
        );
    }
    #[test]
    fn exited_leader_cleanup_terminates_still_running_grandchild() {
        struct Guard(Option<Child>);
        impl Drop for Guard {
            fn drop(&mut self) {
                if let Some(mut child) = self.0.take() {
                    terminate_group(&mut child);
                    let _ = child.wait();
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 30 & printf '%s\n' \"$!\" > \"$DESCENDANT_PID_FILE\"; exit 0",
            ])
            .env("DESCENDANT_PID_FILE", &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut guard = Guard(Some(child));
        let began = Instant::now();
        while !exited_without_reaping(guard.0.as_ref().unwrap()).unwrap() {
            assert!(began.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        let descendant: i32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(descendant, 0) },
            0,
            "grandchild did not remain alive after its launcher exited"
        );
        assert!(finish_exited_group(guard.0.as_mut().unwrap())
            .unwrap()
            .success());
        guard.0.take();
        let stopped = Instant::now();
        loop {
            let output = Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", &descendant.to_string()])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&output.stdout);
            let state = state.trim();
            if state.is_empty() || state.starts_with('Z') {
                break;
            }
            assert!(
                stopped.elapsed() < Duration::from_secs(2),
                "grandchild still running: {state}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn private_fontconfig_escapes_paths_and_changes_cache_namespace() {
        let config = font_configuration(
            &[PathBuf::from("/Users/test & user/Library/Fonts")],
            Path::new("/tmp/private <cache>"),
        );
        assert!(config.contains("test &amp; user"));
        assert!(config.contains("private &lt;cache&gt;"));
        assert!(config.contains("Arial Unicode MS"));
        assert!(!config.contains("<include"));
        let old = Sha256::digest(b"fixture");
        let mut configured = Sha256::new();
        configured.update(RENDERER_CONFIG_VERSION.as_bytes());
        configured.update([0]);
        configured.update(b"fixture");
        assert_ne!(old, configured.finalize());
    }
}
