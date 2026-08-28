//! Disposable Portable sessions.
//!
//! Each Portable process owns one fresh directory under
//! `<system temp>/ArcScanPortable/sessions/<uuid>/`. An exclusive lock proves a
//! session is active. A strict marker proves ArcScan ownership before cleanup.
//! Nothing here consults or writes beside the executable.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::runtime::{
    valid_session_id, PortableError, PortableLayout, ACTIVE_LOCK_FILE, DATABASE_FILE,
    OWNERSHIP_MARKER_FILE, WEBVIEW_DIR,
};
#[cfg(test)]
use crate::runtime::{NAMESPACE_LOCK_FILE, PORTABLE_NAMESPACE_DIR, PORTABLE_SESSIONS_DIR};

const MARKER_PRODUCT: &str = "ArcScan";
const MARKER_KIND: &str = "portable-session";
const MARKER_FORMAT: u32 = 1;
const MAX_MARKER_BYTES: u64 = 4096;
const NAMESPACE_LOCK_ATTEMPTS: usize = 200;
const NAMESPACE_LOCK_DELAY: Duration = Duration::from_millis(10);
#[cfg(any(feature = "portable", test))]
const CLEANUP_HELPER_ARG: &str = "--arcscan-portable-cleanup";
#[cfg(any(all(feature = "portable", windows), test))]
const CLEANUP_HELPER_ATTEMPTS: usize = 50;
#[cfg(any(all(feature = "portable", windows), test))]
const CLEANUP_HELPER_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnershipMarker {
    product: String,
    kind: String,
    format: u32,
    session_id: String,
    created_at: String,
    process_id: u32,
}

impl OwnershipMarker {
    fn new(session_id: &str) -> Self {
        OwnershipMarker {
            product: MARKER_PRODUCT.into(),
            kind: MARKER_KIND.into(),
            format: MARKER_FORMAT,
            session_id: session_id.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            process_id: std::process::id(),
        }
    }

    fn is_valid_for(&self, session_id: &str) -> bool {
        self.product == MARKER_PRODUCT
            && self.kind == MARKER_KIND
            && self.format == MARKER_FORMAT
            && self.session_id == session_id
            && self.process_id != 0
            && chrono::DateTime::parse_from_rfc3339(&self.created_at).is_ok()
    }
}

/// Holds the active-session lock for the entire Portable process lifetime.
#[derive(Debug)]
pub struct PortableSession {
    pub layout: PortableLayout,
    _active_lock: File,
}

impl PortableSession {
    pub fn start() -> Result<Self, PortableError> {
        Self::start_in(&std::env::temp_dir())
    }

    pub(crate) fn start_in(system_temp: &Path) -> Result<Self, PortableError> {
        let roots = namespace_layout(system_temp);
        let _namespace = prepare_namespace(&roots)?;
        let session = create_session(&roots)?;

        // Cleanup is intentionally non-fatal. A locked WebView profile, a
        // malformed unknown folder, or an antivirus race must not make the new
        // independent session unavailable.
        let report = cleanup_stale_locked(&roots.sessions_root, Some(&session.layout.session_id));
        if report.failed > 0 {
            eprintln!(
                "ArcScan Portable could not clean {} stale temporary session(s); they will be retried later.",
                report.failed
            );
        }
        Ok(session)
    }

    #[cfg(any(feature = "portable", test))]
    pub fn cleanup_handle(&self) -> PortableCleanup {
        PortableCleanup {
            layout: self.layout.clone(),
        }
    }
}

/// Path-only cleanup token retained outside Tauri until the event loop returns.
/// On Windows it starts the same executable in a narrow internal helper mode;
/// that helper waits for this process to exit before touching WebView2 files.
#[derive(Debug, Clone)]
#[cfg(any(feature = "portable", test))]
pub struct PortableCleanup {
    layout: PortableLayout,
}

#[cfg(any(feature = "portable", test))]
impl PortableCleanup {
    pub fn cleanup(&self) -> Result<bool, String> {
        let _namespace = prepare_namespace(&self.layout).map_err(|e| e.to_string())?;
        match remove_owned_session(&self.layout.sessions_root, &self.layout.session_root) {
            CleanupDisposition::Removed => Ok(true),
            CleanupDisposition::Active => Ok(false),
            CleanupDisposition::Ignored(reason) | CleanupDisposition::Failed(reason) => Err(reason),
        }
    }

    /// Start a no-window copy of this Portable executable that waits for the
    /// current process to exit, then cleans only this exact owned session.
    #[cfg(all(feature = "portable", windows))]
    pub fn spawn_after_process_exit(&self) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        let process_id = std::process::id();
        let marker = read_marker(&self.layout.ownership_marker_path)?;
        if !marker.is_valid_for(&self.layout.session_id) || marker.process_id != process_id {
            return Err("the active session marker does not belong to this process".into());
        }

        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve the Portable executable: {error}"))?;
        Command::new(executable)
            .arg(CLEANUP_HELPER_ARG)
            .arg(&self.layout.session_id)
            .arg(process_id.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("could not start the Portable cleanup helper: {error}"))
    }
}

/// Intercept the private cleanup-helper invocation before Portable startup can
/// create another session. The helper accepts no path: only a compact session
/// id beneath this process's system-temp namespace and the creator PID recorded
/// in that session's marker.
#[cfg(feature = "portable")]
pub(crate) fn run_cleanup_helper_if_requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    let Some(mode) = args.next() else {
        return false;
    };
    if mode != std::ffi::OsStr::new(CLEANUP_HELPER_ARG) {
        return false;
    }

    let result = (|| -> Result<(), String> {
        let session_id = args
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or("cleanup helper is missing a UTF-8 session id")?;
        if !valid_session_id(&session_id) {
            return Err("cleanup helper session id is not a compact UUID".into());
        }
        let process_id = args
            .next()
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0)
            .ok_or("cleanup helper creator PID is invalid")?;
        if args.next().is_some() {
            return Err("cleanup helper received unexpected arguments".into());
        }

        #[cfg(windows)]
        wait_for_parent_exit(process_id)?;
        #[cfg(not(windows))]
        {
            let _ = process_id;
            Err("Portable cleanup helper mode is Windows-only".into())
        }

        #[cfg(windows)]
        cleanup_helper_session(&std::env::temp_dir(), &session_id, process_id).map(|_| ())
    })();

    if let Err(error) = result {
        eprintln!("ArcScan Portable cleanup helper: {error}");
    }
    true
}

#[cfg(all(feature = "portable", windows))]
fn wait_for_parent_exit(process_id: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    // ERROR_INVALID_PARAMETER means the parent exited before the helper opened
    // its handle. Any other OpenProcess failure is not authority to delete.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(87) {
            Ok(())
        } else {
            Err(format!(
                "could not wait for parent process {process_id}: {error}"
            ))
        };
    }

    let wait = unsafe { WaitForSingleObject(handle, 30_000) };
    unsafe {
        CloseHandle(handle);
    }
    match wait {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(format!(
            "parent process {process_id} did not exit within 30 seconds"
        )),
        value => Err(format!(
            "waiting for parent process {process_id} failed with {value}"
        )),
    }
}

#[cfg(any(all(feature = "portable", windows), test))]
fn cleanup_helper_session(
    system_temp: &Path,
    session_id: &str,
    expected_process_id: u32,
) -> Result<bool, String> {
    if !valid_session_id(session_id) || expected_process_id == 0 {
        return Err("cleanup helper ownership arguments are invalid".into());
    }

    let roots = namespace_layout(system_temp);
    let candidate = roots.sessions_root.join(session_id);
    let mut last_failure = None;
    for attempt in 0..CLEANUP_HELPER_ATTEMPTS {
        let namespace = prepare_namespace(&roots).map_err(|error| error.to_string())?;
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }

        let layout = validate_owned_candidate(&roots.sessions_root, &candidate)?;
        let marker = read_marker(&layout.ownership_marker_path)?;
        if marker.process_id != expected_process_id {
            return Err("cleanup helper creator PID does not match the ownership marker".into());
        }

        match remove_owned_session(&roots.sessions_root, &candidate) {
            CleanupDisposition::Removed => return Ok(true),
            CleanupDisposition::Ignored(reason) => return Err(reason),
            CleanupDisposition::Active => {
                last_failure = Some("the session is still active".to_string())
            }
            CleanupDisposition::Failed(reason) => last_failure = Some(reason),
        }
        drop(namespace);
        if attempt + 1 < CLEANUP_HELPER_ATTEMPTS {
            thread::sleep(CLEANUP_HELPER_DELAY);
        }
    }

    Err(last_failure.unwrap_or_else(|| "cleanup helper exhausted its retries".into()))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: usize,
    pub active: usize,
    pub ignored: usize,
    pub failed: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum CleanupDisposition {
    Removed,
    Active,
    Ignored(String),
    Failed(String),
}

struct NamespaceGuard {
    _lock: File,
}

fn namespace_layout(system_temp: &Path) -> PortableLayout {
    PortableLayout::for_session(system_temp, "00000000000040008000000000000000")
}

fn portable_error(path: &Path, detail: impl Into<String>) -> PortableError {
    PortableError::TemporarySessionUnavailable {
        path: path.display().to_string(),
        detail: detail.into(),
    }
}

fn prepare_namespace(layout: &PortableLayout) -> Result<NamespaceGuard, PortableError> {
    create_plain_directory(&layout.namespace_root)?;
    create_plain_directory(&layout.sessions_root)?;

    let file = open_or_create_plain_file(&layout.namespace_lock_path)?;
    for attempt in 0..NAMESPACE_LOCK_ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return Ok(NamespaceGuard { _lock: file }),
            Err(fs::TryLockError::WouldBlock) if attempt + 1 < NAMESPACE_LOCK_ATTEMPTS => {
                thread::sleep(NAMESPACE_LOCK_DELAY);
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(portable_error(
                    &layout.namespace_lock_path,
                    "another Portable startup kept the session namespace busy",
                ));
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(portable_error(
                    &layout.namespace_lock_path,
                    error.to_string(),
                ));
            }
        }
    }
    unreachable!("bounded namespace lock loop always returns")
}

fn create_plain_directory(path: &Path) -> Result<(), PortableError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(portable_error(path, error.to_string())),
    }
    validate_plain_directory(path).map_err(|detail| portable_error(path, detail))
}

fn open_or_create_plain_file(path: &Path) -> Result<File, PortableError> {
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => {
            validate_plain_file(path).map_err(|detail| portable_error(path, detail))?;
            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_plain_file(path).map_err(|detail| portable_error(path, detail))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| portable_error(path, error.to_string()))
        }
        Err(error) => Err(portable_error(path, error.to_string())),
    }
}

fn create_session(roots: &PortableLayout) -> Result<PortableSession, PortableError> {
    for _ in 0..32 {
        let session_id = Uuid::new_v4().simple().to_string();
        let layout = PortableLayout::from_sessions_root(roots.sessions_root.clone(), &session_id);
        match fs::create_dir(&layout.session_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(portable_error(&layout.session_root, error.to_string())),
        }
        let initialize = || -> Result<File, PortableError> {
            validate_plain_directory(&layout.session_root)
                .map_err(|detail| portable_error(&layout.session_root, detail))?;

            let active_lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&layout.active_lock_path)
                .map_err(|error| portable_error(&layout.active_lock_path, error.to_string()))?;
            active_lock
                .try_lock()
                .map_err(|error| portable_error(&layout.active_lock_path, error.to_string()))?;

            let marker = OwnershipMarker::new(&session_id);
            let bytes = serde_json::to_vec_pretty(&marker).map_err(|error| {
                portable_error(&layout.ownership_marker_path, error.to_string())
            })?;
            let mut marker_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&layout.ownership_marker_path)
                .map_err(|error| {
                    portable_error(&layout.ownership_marker_path, error.to_string())
                })?;
            marker_file
                .write_all(&bytes)
                .and_then(|()| marker_file.sync_all())
                .map_err(|error| {
                    portable_error(&layout.ownership_marker_path, error.to_string())
                })?;
            drop(marker_file);
            Ok(active_lock)
        };

        match initialize() {
            Ok(active_lock) => {
                return Ok(PortableSession {
                    layout,
                    _active_lock: active_lock,
                });
            }
            Err(error) => {
                // This directory was created above with a fresh random name and
                // no application payload can exist yet. Roll back only those
                // three exact paths so a partial marker never becomes an
                // uncollectable stale directory.
                let _ = fs::remove_file(&layout.ownership_marker_path);
                let _ = fs::remove_file(&layout.active_lock_path);
                let _ = fs::remove_dir(&layout.session_root);
                return Err(error);
            }
        }
    }

    Err(portable_error(
        &roots.sessions_root,
        "could not allocate a unique session identifier",
    ))
}

#[cfg(test)]
pub(crate) fn cleanup_stale_sessions_in(
    system_temp: &Path,
) -> Result<CleanupReport, PortableError> {
    let roots = namespace_layout(system_temp);
    let _namespace = prepare_namespace(&roots)?;
    Ok(cleanup_stale_locked(&roots.sessions_root, None))
}

fn cleanup_stale_locked(sessions_root: &Path, current_session_id: Option<&str>) -> CleanupReport {
    let mut report = CleanupReport::default();
    let entries = match fs::read_dir(sessions_root) {
        Ok(entries) => entries,
        Err(_) => {
            report.failed = 1;
            return report;
        }
    };

    for entry in entries {
        let Ok(entry) = entry else {
            report.failed += 1;
            continue;
        };
        let path = entry.path();
        if current_session_id.is_some_and(|id| entry.file_name() == id) {
            report.active += 1;
            continue;
        }
        match remove_owned_session(sessions_root, &path) {
            CleanupDisposition::Removed => report.removed += 1,
            CleanupDisposition::Active => report.active += 1,
            CleanupDisposition::Ignored(_) => report.ignored += 1,
            CleanupDisposition::Failed(_) => report.failed += 1,
        }
    }
    report
}

/// Remove one direct child only after marker, type and activity validation.
///
/// The ownership marker is deliberately removed last. If WebView2 still has a
/// profile file open, the failure leaves the marker in place so a later launch
/// can prove ownership and retry.
fn remove_owned_session(sessions_root: &Path, candidate: &Path) -> CleanupDisposition {
    let layout = match validate_owned_candidate(sessions_root, candidate) {
        Ok(layout) => layout,
        Err(reason) => return CleanupDisposition::Ignored(reason),
    };

    let active_lock = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(&layout.active_lock_path)
    {
        Ok(file) => file,
        Err(error) => return CleanupDisposition::Ignored(error.to_string()),
    };
    match active_lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return CleanupDisposition::Active,
        Err(fs::TryLockError::Error(error)) => {
            return CleanupDisposition::Failed(error.to_string());
        }
    }

    // Revalidate while holding the lock, then validate the whole deletion set
    // before removing its first byte.
    if let Err(reason) = validate_owned_candidate(sessions_root, candidate) {
        return CleanupDisposition::Ignored(reason);
    }
    let payloads = match validated_payloads(&layout) {
        Ok(payloads) => payloads,
        Err(reason) => return CleanupDisposition::Ignored(reason),
    };

    for payload in payloads {
        let result = if payload.is_dir {
            fs::remove_dir_all(&payload.path)
        } else {
            fs::remove_file(&payload.path)
        };
        if let Err(error) = result {
            return CleanupDisposition::Failed(format!("{}: {error}", payload.path.display()));
        }
    }

    drop(active_lock);
    if let Err(error) = fs::remove_file(&layout.active_lock_path) {
        return CleanupDisposition::Failed(format!(
            "{}: {error}",
            layout.active_lock_path.display()
        ));
    }

    let marker_bytes = match fs::read(&layout.ownership_marker_path) {
        Ok(bytes) => bytes,
        Err(error) => return CleanupDisposition::Failed(error.to_string()),
    };
    if let Err(error) = fs::remove_file(&layout.ownership_marker_path) {
        return CleanupDisposition::Failed(format!(
            "{}: {error}",
            layout.ownership_marker_path.display()
        ));
    }
    if let Err(error) = fs::remove_dir(&layout.session_root) {
        // Best effort to restore deletion authority if something raced a new
        // entry into the otherwise empty directory.
        let _ = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&layout.ownership_marker_path)
            .and_then(|mut file| file.write_all(&marker_bytes));
        return CleanupDisposition::Failed(format!("{}: {error}", layout.session_root.display()));
    }

    CleanupDisposition::Removed
}

fn validate_owned_candidate(
    sessions_root: &Path,
    candidate: &Path,
) -> Result<PortableLayout, String> {
    if candidate.parent() != Some(sessions_root) {
        return Err("cleanup target is not a direct session child".into());
    }
    let session_id = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("session name is not valid UTF-8")?;
    if !valid_session_id(session_id) {
        return Err("session name is not a compact UUID".into());
    }

    validate_plain_directory(candidate)?;
    let layout = PortableLayout::from_sessions_root(sessions_root.to_path_buf(), session_id);
    validate_plain_file(&layout.ownership_marker_path)?;
    validate_plain_file(&layout.active_lock_path)?;

    let marker = read_marker(&layout.ownership_marker_path)?;
    if !marker.is_valid_for(session_id) {
        return Err("ownership marker does not match this session".into());
    }
    Ok(layout)
}

fn read_marker(path: &Path) -> Result<OwnershipMarker, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_MARKER_BYTES {
        return Err("ownership marker is oversized".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err("ownership marker is oversized".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid ownership marker: {error}"))
}

struct Payload {
    path: PathBuf,
    is_dir: bool,
}

fn validated_payloads(layout: &PortableLayout) -> Result<Vec<Payload>, String> {
    let mut payloads = Vec::new();
    for entry in fs::read_dir(&layout.session_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or("session entry name is not valid UTF-8")?;
        let path = entry.path();
        match name {
            OWNERSHIP_MARKER_FILE | ACTIVE_LOCK_FILE => continue,
            DATABASE_FILE | "arcscan.db-wal" | "arcscan.db-shm" | "arcscan.db-journal" => {
                validate_plain_file(&path)?;
                payloads.push(Payload {
                    path,
                    is_dir: false,
                });
            }
            WEBVIEW_DIR => {
                validate_plain_directory(&path)?;
                validate_tree_without_links(&path)?;
                payloads.push(Payload { path, is_dir: true });
            }
            _ => return Err(format!("unknown session entry {name}")),
        }
    }
    Ok(payloads)
}

fn validate_tree_without_links(root: &Path) -> Result<(), String> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(format!("{} is a link or reparse point", path.display()));
        }
        if metadata.is_file() {
            continue;
        }
        if !metadata.is_dir() {
            return Err(format!("{} has an unexpected file type", path.display()));
        }
        for entry in fs::read_dir(&path).map_err(|error| error.to_string())? {
            stack.push(entry.map_err(|error| error.to_string())?.path());
        }
    }
    Ok(())
}

fn validate_plain_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err("expected an ordinary directory, not a link or reparse point".into());
    }
    Ok(())
}

fn validate_plain_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err("expected an ordinary file, not a link or reparse point".into());
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "arcscan-disposable-{tag}-{}-{}",
                std::process::id(),
                Uuid::new_v4().simple()
            ));
            fs::create_dir(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_session_has_a_valid_marker_lock_and_no_database_yet() {
        let temp = TempDir::new("create");
        let session = PortableSession::start_in(temp.path()).unwrap();
        assert!(valid_session_id(&session.layout.session_id));
        assert!(session.layout.session_root.is_dir());
        assert!(session.layout.ownership_marker_path.is_file());
        assert!(session.layout.active_lock_path.is_file());
        assert!(!session.layout.database_path.exists());
        assert!(!session.layout.webview_data_path.exists());
        assert!(session.layout.session_root.starts_with(
            temp.path()
                .join(PORTABLE_NAMESPACE_DIR)
                .join(PORTABLE_SESSIONS_DIR)
        ));
    }

    #[test]
    fn two_simultaneous_sessions_from_the_same_temp_root_are_independent() {
        let temp = TempDir::new("two");
        let a = PortableSession::start_in(temp.path()).unwrap();
        let b = PortableSession::start_in(temp.path()).unwrap();
        assert_ne!(a.layout.session_root, b.layout.session_root);
        assert_ne!(a.layout.database_path, b.layout.database_path);
        assert_ne!(a.layout.webview_data_path, b.layout.webview_data_path);
        assert!(a.layout.session_root.exists());
        assert!(b.layout.session_root.exists());
    }

    #[test]
    fn concurrent_creation_is_serialized_but_sessions_are_not_shared() {
        let temp = TempDir::new("race");
        let root_a = temp.path().to_path_buf();
        let root_b = root_a.clone();
        let a = thread::spawn(move || PortableSession::start_in(&root_a).unwrap());
        let b = thread::spawn(move || PortableSession::start_in(&root_b).unwrap());
        let a = a.join().unwrap();
        let b = b.join().unwrap();
        assert_ne!(a.layout.session_id, b.layout.session_id);
        assert!(a.layout.session_root.exists());
        assert!(b.layout.session_root.exists());
    }

    #[test]
    fn active_sessions_survive_stale_cleanup_and_unlocked_sessions_do_not() {
        let temp = TempDir::new("stale");
        let stale = PortableSession::start_in(temp.path()).unwrap();
        let stale_root = stale.layout.session_root.clone();
        drop(stale);

        let active = PortableSession::start_in(temp.path()).unwrap();
        let active_root = active.layout.session_root.clone();
        assert!(
            !stale_root.exists(),
            "the next startup should remove stale state"
        );
        assert!(active_root.exists());

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert_eq!(report.active, 1);
        assert!(active_root.exists());
    }

    #[test]
    fn normal_cleanup_removes_the_whole_owned_session() {
        let temp = TempDir::new("normal");
        let session = PortableSession::start_in(temp.path()).unwrap();
        fs::write(&session.layout.database_path, b"sqlite fixture").unwrap();
        fs::create_dir(&session.layout.webview_data_path).unwrap();
        fs::write(session.layout.webview_data_path.join("prefs"), b"dark").unwrap();
        let root = session.layout.session_root.clone();
        let cleanup = session.cleanup_handle();
        drop(session);
        assert!(cleanup.cleanup().unwrap());
        assert!(!root.exists());
    }

    #[test]
    fn cleanup_helper_requires_the_marker_creator_and_removes_only_that_session() {
        let temp = TempDir::new("helper");
        let session = PortableSession::start_in(temp.path()).unwrap();
        let session_id = session.layout.session_id.clone();
        let root = session.layout.session_root.clone();
        let process_id = std::process::id();
        drop(session);

        assert!(cleanup_helper_session(temp.path(), &session_id, process_id).unwrap());
        assert!(!root.exists());
    }

    #[test]
    fn cleanup_helper_refuses_a_creator_pid_that_does_not_match_the_marker() {
        let temp = TempDir::new("helper-pid");
        let session = PortableSession::start_in(temp.path()).unwrap();
        let session_id = session.layout.session_id.clone();
        let root = session.layout.session_root.clone();
        let wrong_process_id = if std::process::id() == 1 {
            2
        } else {
            std::process::id() - 1
        };
        drop(session);

        assert!(cleanup_helper_session(temp.path(), &session_id, wrong_process_id).is_err());
        assert!(root.exists());
    }

    #[test]
    fn malformed_and_mismatched_markers_are_preserved() {
        let temp = TempDir::new("bad-marker");
        let malformed = PortableSession::start_in(temp.path()).unwrap();
        let malformed_root = malformed.layout.session_root.clone();
        let malformed_marker = malformed.layout.ownership_marker_path.clone();
        drop(malformed);
        fs::write(&malformed_marker, b"not json").unwrap();

        let mismatched = PortableSession::start_in(temp.path()).unwrap();
        let mismatched_root = mismatched.layout.session_root.clone();
        let mismatched_marker = mismatched.layout.ownership_marker_path.clone();
        drop(mismatched);
        let wrong = OwnershipMarker::new("fedcba9876544abc8fedcba987654321");
        fs::write(&mismatched_marker, serde_json::to_vec(&wrong).unwrap()).unwrap();

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert!(report.ignored >= 2);
        assert!(malformed_root.exists());
        assert!(mismatched_root.exists());
    }

    #[test]
    fn oversized_and_path_bearing_markers_are_preserved() {
        let temp = TempDir::new("marker-shape");
        let oversized = PortableSession::start_in(temp.path()).unwrap();
        let oversized_root = oversized.layout.session_root.clone();
        let oversized_marker = oversized.layout.ownership_marker_path.clone();
        drop(oversized);
        fs::write(&oversized_marker, vec![b'x'; MAX_MARKER_BYTES as usize + 1]).unwrap();

        let path_bearing = PortableSession::start_in(temp.path()).unwrap();
        let path_bearing_root = path_bearing.layout.session_root.clone();
        let path_bearing_marker = path_bearing.layout.ownership_marker_path.clone();
        let marker = OwnershipMarker::new(&path_bearing.layout.session_id);
        drop(path_bearing);
        let mut value = serde_json::to_value(marker).unwrap();
        value["delete"] = serde_json::json!("C:\\\\not-ours");
        fs::write(&path_bearing_marker, serde_json::to_vec(&value).unwrap()).unwrap();

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert!(report.ignored >= 2);
        assert!(oversized_root.exists());
        assert!(path_bearing_root.exists());
    }

    #[test]
    fn unknown_folders_and_generic_temp_data_are_never_deleted() {
        let temp = TempDir::new("unknown");
        let sessions = temp
            .path()
            .join(PORTABLE_NAMESPACE_DIR)
            .join(PORTABLE_SESSIONS_DIR);
        fs::create_dir_all(&sessions).unwrap();
        let unknown = sessions.join("0123456789ab4def8123456789abcdef");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("important.txt"), b"not ArcScan").unwrap();
        let generic = temp.path().join("unrelated-project");
        fs::create_dir(&generic).unwrap();
        fs::write(generic.join("keep.txt"), b"keep").unwrap();

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert_eq!(report.removed, 0);
        assert!(unknown.join("important.txt").exists());
        assert!(generic.join("keep.txt").exists());
    }

    #[test]
    fn an_unknown_entry_leaves_the_marker_for_a_future_safe_retry() {
        let temp = TempDir::new("marker-last");
        let session = PortableSession::start_in(temp.path()).unwrap();
        let root = session.layout.session_root.clone();
        let marker = session.layout.ownership_marker_path.clone();
        fs::write(root.join("explicit-export.csv"), b"keep me").unwrap();
        drop(session);

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert_eq!(report.ignored, 1);
        assert!(
            marker.exists(),
            "marker must survive every incomplete cleanup"
        );
        assert!(root.join("explicit-export.csv").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_webview_is_ignored_and_its_external_target_survives() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("symlink");
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();

        let session = PortableSession::start_in(temp.path()).unwrap();
        let root = session.layout.session_root.clone();
        symlink(&outside, &session.layout.webview_data_path).unwrap();
        drop(session);

        let report = cleanup_stale_sessions_in(temp.path()).unwrap();
        assert_eq!(report.ignored, 1);
        assert!(root.exists());
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn temp_creation_failure_is_fatal_and_does_not_touch_appdata() {
        let temp = TempDir::new("failure");
        let not_a_directory = temp.path().join("not-a-directory");
        fs::write(&not_a_directory, b"block").unwrap();
        let appdata = temp.path().join("AppData");
        fs::create_dir(&appdata).unwrap();
        fs::write(appdata.join("arcscan.db"), b"installed sentinel").unwrap();

        let error = PortableSession::start_in(&not_a_directory).unwrap_err();
        assert!(matches!(
            error,
            PortableError::TemporarySessionUnavailable { .. }
        ));
        assert_eq!(
            fs::read(appdata.join("arcscan.db")).unwrap(),
            b"installed sentinel"
        );
    }

    #[test]
    fn external_csv_json_and_xml_exports_survive_session_cleanup() {
        let temp = TempDir::new("exports");
        let exports = temp.path().join("exports");
        fs::create_dir(&exports).unwrap();
        for name in ["scan.csv", "scan.json", "scan.xml"] {
            fs::write(exports.join(name), format!("valid {name}")).unwrap();
        }

        let session = PortableSession::start_in(temp.path()).unwrap();
        let cleanup = session.cleanup_handle();
        drop(session);
        assert!(cleanup.cleanup().unwrap());
        for name in ["scan.csv", "scan.json", "scan.xml"] {
            assert_eq!(
                fs::read_to_string(exports.join(name)).unwrap(),
                format!("valid {name}")
            );
        }
    }

    #[test]
    fn no_runtime_file_is_created_beside_a_read_only_executable_folder() {
        let temp = TempDir::new("readonly-exe");
        let exe_folder = temp.path().join("toolkit");
        fs::create_dir(&exe_folder).unwrap();
        fs::write(exe_folder.join("ArcScan.exe"), b"fixture").unwrap();

        let session = PortableSession::start_in(temp.path()).unwrap();
        let names: Vec<_> = fs::read_dir(&exe_folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("ArcScan.exe")]);
        assert!(!session.layout.session_root.starts_with(&exe_folder));
    }

    #[test]
    fn marker_constants_and_namespace_lock_are_exact() {
        assert_eq!(NAMESPACE_LOCK_FILE, ".sessions.lock");
        assert_eq!(OWNERSHIP_MARKER_FILE, ".arcscan-portable-session");
        assert_eq!(ACTIVE_LOCK_FILE, ".arcscan-portable-session.lock");
    }
}
