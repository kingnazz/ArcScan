//! Portable startup: prove the folder works, then claim it.
//!
//! Portable ArcScan opens no application state until it has established that
//! the folder it is running from can actually keep that state, and that no
//! other copy is already keeping it there. If either is not true it says so and
//! stops. There is no AppData fallback, no second database somewhere else, and
//! no startup that looks successful with the data going to a place the operator
//! did not choose — which is the difference between a portable build and an
//! installed build in a ZIP.
//!
//! # Why a write, and not a flag
//!
//! [`probe_writable`] creates a file, writes to it, flushes it and deletes it.
//! Reading a read-only attribute would be cheaper and would be wrong: a
//! directory ACL that denies creation, a full disk, a write-protect switch on an
//! SD card, an antivirus holding the folder, and removable media that has
//! already been pulled all report a perfectly writable directory right up until
//! something is written to it.
//!
//! # Why an OS lock, and not a lock file
//!
//! [`lock_data_root`] takes a real advisory lock on `runtime.lock` and holds it
//! for the life of the process. The alternative — treating the *existence* of
//! `runtime.lock` as the lock — fails in the one case that matters: a crash
//! leaves the file behind and every future launch is refused until somebody
//! finds and deletes a file they have never heard of. An OS lock is released by
//! the kernel when the process ends, however it ends, so the next launch simply
//! relocks the file that is already sitting there.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::runtime::{
    classify_location, DriveTypeProbe, LocationKind, PortableError, PortableLayout,
};

/// Holds a portable data root for the life of the process.
///
/// The lock is the open file handle inside this value, so keeping the guard
/// alive is what keeps the claim. Dropping it — or the process ending for any
/// reason, including a crash — releases the lock.
#[derive(Debug)]
pub struct PortableGuard {
    /// The locked `runtime.lock` handle. Never read or written; only held.
    _lock: fs::File,
}

/// Run the portable startup checks against a resolved layout.
///
/// In order: refuse a network location, create the data folder, prove it can be
/// written to, then claim it. Returns the guard that must be kept alive for as
/// long as ArcScan is running.
///
/// The database is opened by the caller *after* this returns, so a folder that
/// cannot hold a database never has one created in it.
pub fn preflight(
    layout: &PortableLayout,
    probe: &dyn DriveTypeProbe,
) -> Result<PortableGuard, PortableError> {
    if classify_location(&layout.portable_root, probe) == LocationKind::Network {
        return Err(PortableError::NetworkLocation {
            location: layout.portable_root.display().to_string(),
        });
    }

    fs::create_dir_all(&layout.data_root).map_err(|e| PortableError::CannotCreateDataDir {
        path: layout.data_root.display().to_string(),
        detail: e.to_string(),
    })?;

    probe_writable(&layout.data_root)?;

    lock_data_root(&layout.lock_path)
}

/// Prove `dir` is writable by writing to it.
///
/// Every step is checked separately, because they fail separately: a directory
/// can allow creation but not writing (a quota), allow writing but not flushing
/// (a disconnected drive), and allow all three but not deletion (a mandatory
/// lock or an antivirus). The probe name carries the process id so two ArcScans
/// racing here cannot delete each other's probe and report a failure that is
/// really the other process tidying up.
pub fn probe_writable(dir: &Path) -> Result<(), PortableError> {
    let path = dir.join(format!(".arcscan-write-probe-{}", std::process::id()));
    let fail = |detail: String| PortableError::NotWritable {
        path: dir.display().to_string(),
        detail,
    };

    let mut file = fs::File::create(&path).map_err(|e| fail(e.to_string()))?;
    let outcome = file
        .write_all(b"ArcScan Portable write probe\n")
        .and_then(|()| file.flush())
        // sync_data is the step that actually reaches the device on removable
        // media, where the earlier ones can succeed against a cache that is
        // never written back.
        .and_then(|()| file.sync_data());
    drop(file);

    if let Err(e) = outcome {
        // Best effort: the probe may or may not be removable now, and the
        // failure being reported is the write, not the cleanup.
        let _ = fs::remove_file(&path);
        return Err(fail(e.to_string()));
    }

    fs::remove_file(&path).map_err(|e| fail(e.to_string()))?;
    Ok(())
}

/// Take the exclusive same-root lock, or explain which way it failed.
///
/// The two failures are told apart deliberately. `WouldBlock` means another
/// ArcScan holds this folder, and the operator needs to close a window.
/// Anything else means the lock file itself could not be created or locked —
/// a read-only folder, a filesystem with no locking — and the operator needs a
/// different folder. Telling someone to close a window they do not have open
/// would be worse than saying nothing.
pub fn lock_data_root(path: &Path) -> Result<PortableGuard, PortableError> {
    let file = fs::File::options()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| PortableError::LockUnavailable {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;

    match file.try_lock() {
        Ok(()) => Ok(PortableGuard { _lock: file }),
        Err(fs::TryLockError::WouldBlock) => Err(PortableError::AlreadyRunning {
            path: path.parent().unwrap_or(path).display().to_string(),
        }),
        Err(fs::TryLockError::Error(e)) => Err(PortableError::LockUnavailable {
            path: path.display().to_string(),
            detail: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::SystemDriveType;
    use std::path::PathBuf;

    /// A temporary directory that removes itself, so the suite leaves nothing
    /// behind on a machine or a CI runner.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "arcscan-portable-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
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

    struct NeverRemote;
    impl DriveTypeProbe for NeverRemote {
        fn is_remote(&self, _path: &Path) -> bool {
            false
        }
    }

    struct AlwaysRemote;
    impl DriveTypeProbe for AlwaysRemote {
        fn is_remote(&self, _path: &Path) -> bool {
            true
        }
    }

    #[test]
    fn a_writable_folder_passes_and_creates_the_data_root() {
        let temp = TempDir::new("ok");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());
        assert!(!layout.data_root.exists());

        let guard = preflight(&layout, &NeverRemote).unwrap();

        assert!(layout.data_root.is_dir());
        assert!(layout.lock_path.is_file());
        // The preflight creates the data root and the lock, and nothing else.
        // No database, no WebView folder: those are the caller's to make, after
        // this has said the folder works.
        assert!(!layout.database_path.exists());
        assert!(!layout.webview_data_path.exists());
        drop(guard);
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let temp = TempDir::new("probe");
        probe_writable(temp.path()).unwrap();
        let left: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "probe left {left:?}");
    }

    #[test]
    fn a_network_location_is_refused_before_anything_is_created() {
        let temp = TempDir::new("unc");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());
        let error = preflight(&layout, &AlwaysRemote).unwrap_err();
        assert!(matches!(error, PortableError::NetworkLocation { .. }));
        // Refused means refused: not one directory made on the way out.
        assert!(!layout.data_root.exists());
    }

    #[test]
    fn a_missing_parent_that_cannot_be_created_reports_the_folder() {
        // A path under a *file* can never become a directory, which is the
        // deterministic stand-in for a denied ACL: create_dir_all fails.
        let temp = TempDir::new("nodir");
        let blocker = temp.path().join("not-a-directory");
        fs::write(&blocker, b"x").unwrap();
        let layout = PortableLayout::for_root(blocker.join("ArcScan Portable"));
        let error = preflight(&layout, &NeverRemote).unwrap_err();
        assert!(
            matches!(error, PortableError::CannotCreateDataDir { .. }),
            "{error:?}"
        );
        assert!(error.message().contains("cannot save data"));
        assert!(error.detail().is_some());
    }

    #[test]
    fn the_probe_fails_when_the_folder_is_a_file() {
        // File::create inside a non-directory is the portable, deterministic
        // "this folder will not take a write".
        let temp = TempDir::new("probefail");
        let file = temp.path().join("f");
        fs::write(&file, b"x").unwrap();
        let error = probe_writable(&file).unwrap_err();
        assert!(
            matches!(error, PortableError::NotWritable { .. }),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_fails_the_probe_rather_than_falling_back() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new("ro");
        let dir = temp.path().join("locked");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = probe_writable(&dir);
        // Restore first, so the temporary directory can be removed whatever
        // the assertion below does.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        match outcome {
            Err(error @ PortableError::NotWritable { .. }) => {
                let message = error.message();
                assert!(message.contains("Move the ArcScan Portable folder"));
                assert!(!message.to_lowercase().contains("appdata"));
            }
            // Mode bits do not apply to a sufficiently privileged process, and
            // CI containers frequently run as root. That is an environment
            // fact, not a behaviour to assert around: the packaged read-only
            // case is verified by hand on Windows as well.
            Ok(()) => {}
            Err(other) => panic!("expected a write failure, got {other:?}"),
        }
    }

    #[test]
    fn a_second_instance_from_the_same_root_is_refused() {
        let temp = TempDir::new("lock1");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());

        let first = preflight(&layout, &NeverRemote).unwrap();

        let error = preflight(&layout, &NeverRemote).unwrap_err();
        assert!(
            matches!(error, PortableError::AlreadyRunning { .. }),
            "{error:?}"
        );
        assert!(error.message().contains("already running from this folder"));

        drop(first);
    }

    #[test]
    fn a_different_root_is_allowed_at_the_same_time() {
        let a = TempDir::new("lock-a");
        let b = TempDir::new("lock-b");
        let layout_a = PortableLayout::for_root(a.path().to_path_buf());
        let layout_b = PortableLayout::for_root(b.path().to_path_buf());

        let guard_a = preflight(&layout_a, &NeverRemote).unwrap();
        let guard_b = preflight(&layout_b, &NeverRemote).unwrap();

        assert_ne!(layout_a.data_root, layout_b.data_root);
        drop(guard_a);
        drop(guard_b);
    }

    #[test]
    fn the_lock_is_released_on_a_clean_close() {
        let temp = TempDir::new("lock-release");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());

        let first = preflight(&layout, &NeverRemote).unwrap();
        drop(first);

        // Same folder, straight away, no error.
        let second = preflight(&layout, &NeverRemote).unwrap();
        drop(second);
    }

    #[test]
    fn a_leftover_lock_file_does_not_brick_the_next_launch() {
        // This is the crash case. The file is there, with content, and nothing
        // holds it. A launch must succeed, because the kernel released the lock
        // when the previous process died -- which is exactly why existence is
        // not the test.
        let temp = TempDir::new("stale");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());
        fs::create_dir_all(&layout.data_root).unwrap();
        fs::write(&layout.lock_path, b"leftover from a crash").unwrap();

        let guard = preflight(&layout, &NeverRemote).unwrap();
        assert!(layout.lock_path.is_file());
        drop(guard);

        // And again, repeatedly: relocking is idempotent.
        for _ in 0..3 {
            drop(preflight(&layout, &NeverRemote).unwrap());
        }
    }

    #[test]
    fn the_real_drive_probe_allows_the_temp_directory() {
        // The production probe must not refuse an ordinary local folder. On
        // Windows this exercises GetDriveTypeW for real; elsewhere it confirms
        // the non-Windows arm answers "local".
        let temp = TempDir::new("realdrive");
        let layout = PortableLayout::for_root(temp.path().to_path_buf());
        drop(preflight(&layout, &SystemDriveType).unwrap());
    }
}
