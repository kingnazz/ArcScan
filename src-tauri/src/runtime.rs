//! Which edition of ArcScan this binary is, and where its data lives.
//!
//! # One decision, made at compile time
//!
//! ArcScan ships in two editions from one codebase. The installed edition keeps
//! its database under the normal application-data directory, exactly as every
//! release before 1.8.4 did. The portable edition keeps it in an `ArcScanData`
//! folder beside the executable, so a copy of ArcScan on a USB stick carries
//! its own history, names, notes and preferences.
//!
//! The edition is decided by the `portable` Cargo feature and by nothing else.
//! It is deliberately *not* inferred from the folder the executable sits in,
//! because every runtime signal available to infer it from is one a user can
//! change by accident:
//!
//! * a marker file or an `ArcScanData` folder means an installed ArcScan starts
//!   reading a different database because somebody created a folder with an
//!   unlucky name, and a portable ArcScan starts reading the installed one
//!   because the folder was deleted or the antivirus quarantined it;
//! * the executable's path means renaming `ArcScan.exe` or moving it one
//!   directory up relocates the database;
//! * an environment variable means a parent process decides where a scanner
//!   writes its inventory.
//!
//! All three are silent data-location changes, which is the single thing a
//! persistence design must never do. A compiled-in constant cannot be any of
//! them: the installed binary is installed wherever it is put and whatever
//! surrounds it, and the portable binary is portable.
//!
//! # What is testable here
//!
//! Path resolution is pure: [`PortableLayout::for_executable`] takes an
//! executable path and returns the folder layout, so the interesting cases
//! (spaces, non-ASCII names, deep nesting, a root-level executable) are unit
//! tests rather than manual checks. The parts that must touch a filesystem —
//! the write probe and the lock — take a directory, so they run against a
//! temporary directory. Network-location classification takes its drive-type
//! answer through a trait, so a remote drive is testable on a machine that has
//! none.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Name of the folder holding a portable copy's data, beside the executable.
pub const PORTABLE_DATA_DIR: &str = "ArcScanData";
/// The database file name, identical in both editions.
pub const DATABASE_FILE: &str = "arcscan.db";
/// The WebView profile directory inside a portable data root.
pub const WEBVIEW_DIR: &str = "WebView";
/// The same-root lock file inside a portable data root.
pub const LOCK_FILE: &str = "runtime.lock";

/// Which edition this binary is. Compiled in; see the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Installed,
    Portable,
}

impl Edition {
    /// The edition of *this* binary. A constant, evaluated at compile time.
    pub const fn current() -> Self {
        if cfg!(feature = "portable") {
            Edition::Portable
        } else {
            Edition::Installed
        }
    }

    pub const fn is_portable(self) -> bool {
        matches!(self, Edition::Portable)
    }
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Edition::Installed => "installed",
            Edition::Portable => "portable",
        })
    }
}

/// How this edition handles updates.
///
/// `Installer` is the behaviour every release so far has had: check the signed
/// feed, download, install, relaunch. `Manual` is portable mode, where there is
/// no installer to run and replacing the application files is the operator's
/// deliberate act. The portable build does not merely report `Manual` — it does
/// not compile the updater plugin at all, so there is no install path behind
/// the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdaterMode {
    Installer,
    Manual,
}

impl UpdaterMode {
    pub const fn current() -> Self {
        match Edition::current() {
            Edition::Installed => UpdaterMode::Installer,
            Edition::Portable => UpdaterMode::Manual,
        }
    }
}

/// The platform label shown in Settings, from the compile target.
///
/// Not from the user agent. A WebView's user agent describes the machine, and
/// the point of the edition line is to say which *build* is running -- which is
/// the one thing a machine description cannot tell you when an x64 build is
/// running on an ARM64 Windows box under emulation.
pub const fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Desktop"
    }
}

/// The architecture label shown in Settings. Taken from the compile target, so
/// an ARM64 build says ARM64 even when it is running beside an x64 installer.
pub const fn architecture() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "ARM64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    }
}

/// Where a portable copy keeps everything, derived from its executable.
///
/// Pure: no filesystem access, no environment, no current directory. Every
/// field is a fixed child of the executable's own folder, so there is no input
/// through which a caller could aim any of them somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableLayout {
    pub portable_root: PathBuf,
    pub data_root: PathBuf,
    pub database_path: PathBuf,
    pub webview_data_path: PathBuf,
    pub lock_path: PathBuf,
}

impl PortableLayout {
    /// Resolve the layout for an executable at `exe`.
    ///
    /// Fails when the executable has no parent directory, which in practice
    /// means it was handed a relative bare filename rather than the real path
    /// `current_exe()` returns.
    pub fn for_executable(exe: &Path) -> Result<Self, PortableError> {
        let portable_root = exe
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(PortableError::NoExecutableDirectory)?
            .to_path_buf();
        Ok(Self::for_root(portable_root))
    }

    /// Resolve the layout for a known portable root. Used by the tests and by
    /// [`Self::for_executable`]; the children are always the same fixed names.
    pub fn for_root(portable_root: PathBuf) -> Self {
        let data_root = portable_root.join(PORTABLE_DATA_DIR);
        PortableLayout {
            database_path: data_root.join(DATABASE_FILE),
            webview_data_path: data_root.join(WEBVIEW_DIR),
            lock_path: data_root.join(LOCK_FILE),
            data_root,
            portable_root,
        }
    }
}

/// What the frontend is told about the running edition.
///
/// A deliberately small, flat, display-oriented shape. It carries the data root
/// as one already-formatted string and **no other path**, because the frontend
/// has no business reconstructing a portable path: every app-owned path
/// decision is made here, in Rust, at startup, and the interface's job is to
/// show the answer and offer to copy it.
///
/// `writable` is not "the operator has permission" in the abstract. It is "the
/// startup probe wrote a file into the data root and deleted it again", which is
/// the only form of the question worth answering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeInfo {
    pub edition: Edition,
    pub version: String,
    pub platform: &'static str,
    pub architecture: &'static str,
    /// The data root, formatted for display and for `Copy data path`.
    pub data_root: String,
    /// Whether the startup write probe succeeded. Always true once a portable
    /// build has started at all, since it refuses to start otherwise; kept
    /// because the interface says so explicitly and an installed build answers
    /// it too.
    pub writable: bool,
    pub updater_mode: UpdaterMode,
}

impl RuntimeInfo {
    pub fn new(data_root: &Path, writable: bool) -> Self {
        RuntimeInfo {
            edition: Edition::current(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform(),
            architecture: architecture(),
            data_root: data_root.display().to_string(),
            writable,
            updater_mode: UpdaterMode::current(),
        }
    }
}

/// Where this process keeps everything, resolved once during startup.
///
/// The installed edition has no portable root and no explicit WebView
/// directory: it keeps passing nothing to the WebView, exactly as every release
/// before 1.8.4 did, so its profile stays where its profile has always been.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub edition: Edition,
    pub data_root: PathBuf,
    pub database_path: PathBuf,
    /// `Some` only in portable mode.
    pub webview_data_path: Option<PathBuf>,
    /// `Some` only in portable mode.
    pub portable_root: Option<PathBuf>,
}

impl RuntimePaths {
    /// The installed layout: the database directly under the application-data
    /// directory Tauri resolves, and nothing else changed.
    pub fn installed(app_data_dir: PathBuf) -> Self {
        RuntimePaths {
            edition: Edition::Installed,
            database_path: app_data_dir.join(DATABASE_FILE),
            data_root: app_data_dir,
            webview_data_path: None,
            portable_root: None,
        }
    }

    /// The portable layout, from an already-resolved [`PortableLayout`].
    pub fn portable(layout: &PortableLayout) -> Self {
        RuntimePaths {
            edition: Edition::Portable,
            data_root: layout.data_root.clone(),
            database_path: layout.database_path.clone(),
            webview_data_path: Some(layout.webview_data_path.clone()),
            portable_root: Some(layout.portable_root.clone()),
        }
    }

    pub fn info(&self) -> RuntimeInfo {
        RuntimeInfo::new(&self.data_root, true)
    }
}

/// Every way portable startup can refuse to run.
///
/// Each carries the operator-facing sentence ArcScan shows and, where there is
/// one, the underlying technical detail — shown separately, because "ArcScan
/// Portable cannot save data in this folder" is what someone needs to read
/// first and `os error 5` is what they need to send on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortableError {
    /// `current_exe()` gave a path with no directory component.
    NoExecutableDirectory,
    /// The portable folder is on a network share (§7 of the architecture doc).
    NetworkLocation { location: String },
    /// `ArcScanData` could not be created.
    CannotCreateDataDir { path: String, detail: String },
    /// `ArcScanData` exists but a probe file could not be written to it.
    NotWritable { path: String, detail: String },
    /// Another ArcScan already holds this data root's lock.
    AlreadyRunning { path: String },
    /// The lock file itself could not be created or locked.
    LockUnavailable { path: String, detail: String },
    /// The database is there and writable but would not open.
    DatabaseUnavailable { path: String, detail: String },
}

impl PortableError {
    /// The headline shown to the operator. Says what happened and what to do,
    /// in that order, and never mentions a fallback because there is not one.
    pub fn message(&self) -> String {
        match self {
            PortableError::NoExecutableDirectory => "ArcScan Portable could not work out which \
                 folder it is running from.\n\nExtract the ArcScan Portable ZIP to a folder and \
                 run ArcScan.exe from there."
                .into(),
            PortableError::NetworkLocation { .. } => "ArcScan Portable is running from a network \
                 location.\n\nCopy the ArcScan Portable folder to a local or removable drive and \
                 run it again."
                .into(),
            PortableError::CannotCreateDataDir { .. }
            | PortableError::NotWritable { .. }
            | PortableError::DatabaseUnavailable { .. } => "ArcScan Portable cannot save data in \
                 this folder.\n\nMove the ArcScan Portable folder to a writable local folder or \
                 USB drive and try again."
                .into(),
            PortableError::AlreadyRunning { .. } => {
                "ArcScan Portable is already running from this \
                 folder.\n\nClose the other ArcScan window before starting another copy from the \
                 same portable folder."
                    .into()
            }
            PortableError::LockUnavailable { .. } => "ArcScan Portable could not check whether \
                 another copy is already using this folder.\n\nMove the ArcScan Portable folder \
                 to a writable local folder or USB drive and try again."
                .into(),
        }
    }

    /// The technical line, shown under the headline for a bug report. Contains
    /// the path involved and the operating system's own words, nothing else.
    pub fn detail(&self) -> Option<String> {
        match self {
            PortableError::NoExecutableDirectory => None,
            PortableError::NetworkLocation { location } => Some(format!("Location: {location}")),
            PortableError::CannotCreateDataDir { path, detail } => {
                Some(format!("Could not create {path}: {detail}"))
            }
            PortableError::NotWritable { path, detail } => {
                Some(format!("Could not write to {path}: {detail}"))
            }
            PortableError::AlreadyRunning { path } => Some(format!("Data folder: {path}")),
            PortableError::LockUnavailable { path, detail } => {
                Some(format!("Could not lock {path}: {detail}"))
            }
            PortableError::DatabaseUnavailable { path, detail } => {
                Some(format!("Could not open {path}: {detail}"))
            }
        }
    }
}

impl fmt::Display for PortableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail() {
            Some(detail) => write!(f, "{} ({detail})", self.message().replace('\n', " ")),
            None => f.write_str(&self.message().replace('\n', " ")),
        }
    }
}

/// What ArcScan reports about a location it might run from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    /// A fixed disk, removable media, a RAM disk — anything local.
    Local,
    /// A UNC path or a drive letter Windows reports as remote.
    Network,
}

/// Answers "is this drive letter a network drive". A trait so a remote drive is
/// testable on a machine that has none, and so the Windows API call has exactly
/// one place it can be made from.
pub trait DriveTypeProbe {
    /// `true` when the drive holding `path` is a network drive.
    fn is_remote(&self, path: &Path) -> bool;
}

/// The real answer, from `GetDriveTypeW` on Windows and never on anything else.
pub struct SystemDriveType;

impl DriveTypeProbe for SystemDriveType {
    #[cfg(windows)]
    fn is_remote(&self, path: &Path) -> bool {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, DRIVE_REMOTE};

        // GetDriveTypeW wants a root: "E:\". Anything else and it answers about
        // the current directory, which is not what is being asked.
        let Some(root) = drive_root(path) else {
            return false;
        };
        let wide: Vec<u16> = std::ffi::OsStr::new(&root)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that outlives the
        // call, which is the function's only requirement.
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        kind == DRIVE_REMOTE
    }

    #[cfg(not(windows))]
    fn is_remote(&self, _path: &Path) -> bool {
        // Portable mode ships on Windows only in 1.8.4. On other platforms
        // there is no drive letter to classify, and guessing from a mount table
        // would be a new and untested behaviour on a platform that does not use
        // this code path.
        false
    }
}

/// `E:\Tools\ArcScan` -> `E:\`. `None` for anything that is not a drive-letter
/// path, including UNC paths, which are classified before this is reached.
#[cfg(windows)]
fn drive_root(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(format!("{}:\\", bytes[0] as char));
    }
    None
}

/// Classify a portable folder's location.
///
/// UNC first, by syntax, because a UNC path has no drive letter to ask about:
/// `\\server\share\...` and the verbatim `\\?\UNC\server\share\...` form are
/// both network locations no matter what any API says. Then the drive type,
/// which is what catches a mapped drive such as `Z:` pointing at a share.
pub fn classify_location(path: &Path, probe: &dyn DriveTypeProbe) -> LocationKind {
    if is_unc(path) {
        return LocationKind::Network;
    }
    if probe.is_remote(path) {
        return LocationKind::Network;
    }
    LocationKind::Local
}

/// Whether a path is a UNC path, by shape.
///
/// `\\server\share` and `//server/share` are UNC. `\\?\UNC\server\share` is the
/// verbatim spelling of the same thing and must be caught too. `\\?\C:\...` and
/// `\\.\PhysicalDrive0` are *not* network paths: the first is a verbatim local
/// path, and the second is a device namespace path, which is not somewhere a
/// portable folder can be.
fn is_unc(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let normalised = text.replace('/', "\\");
    if let Some(rest) = normalised.strip_prefix("\\\\") {
        // Verbatim and device prefixes: only the UNC spelling is a share.
        if let Some(after) = rest
            .strip_prefix("?\\")
            .or_else(|| rest.strip_prefix(".\\"))
        {
            let upper = after.to_ascii_uppercase();
            return upper == "UNC" || upper.starts_with("UNC\\");
        }
        // `\\` followed by anything else is a plain UNC path. `\\` alone is not
        // a location at all, and is not treated as one.
        return !rest.is_empty();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_edition_is_the_compiled_in_one() {
        // Whichever way this crate was compiled, the two views of the edition
        // agree and the updater mode follows from it. Nothing consults the
        // filesystem to answer either question.
        let edition = Edition::current();
        assert_eq!(edition.is_portable(), cfg!(feature = "portable"));
        assert_eq!(
            UpdaterMode::current(),
            if edition.is_portable() {
                UpdaterMode::Manual
            } else {
                UpdaterMode::Installer
            }
        );
    }

    #[test]
    fn editions_serialize_as_the_frontend_expects() {
        assert_eq!(
            serde_json::to_string(&Edition::Portable).unwrap(),
            "\"portable\""
        );
        assert_eq!(
            serde_json::to_string(&Edition::Installed).unwrap(),
            "\"installed\""
        );
        assert_eq!(
            serde_json::to_string(&UpdaterMode::Manual).unwrap(),
            "\"manual\""
        );
        assert_eq!(
            serde_json::to_string(&UpdaterMode::Installer).unwrap(),
            "\"installer\""
        );
    }

    #[test]
    fn the_platform_label_is_the_compile_targets() {
        assert!(["Windows", "macOS", "Linux", "Desktop"].contains(&platform()));
        if cfg!(target_os = "windows") {
            assert_eq!(platform(), "Windows");
        }
        if cfg!(target_os = "linux") {
            assert_eq!(platform(), "Linux");
        }
    }

    #[test]
    fn the_architecture_label_is_one_of_the_ones_settings_can_show() {
        assert!(["x64", "ARM64", "x86", "unknown"].contains(&architecture()));
        // And it is the compile target's, not the host's.
        if cfg!(target_arch = "x86_64") {
            assert_eq!(architecture(), "x64");
        }
        if cfg!(target_arch = "aarch64") {
            assert_eq!(architecture(), "ARM64");
        }
    }

    #[test]
    fn the_layout_hangs_off_the_executables_own_folder() {
        let layout =
            PortableLayout::for_executable(Path::new("/opt/tools/arcscan/ArcScan")).unwrap();
        assert_eq!(layout.portable_root, Path::new("/opt/tools/arcscan"));
        assert_eq!(
            layout.data_root,
            Path::new("/opt/tools/arcscan").join("ArcScanData")
        );
        assert_eq!(layout.database_path, layout.data_root.join("arcscan.db"));
        assert_eq!(layout.webview_data_path, layout.data_root.join("WebView"));
        assert_eq!(layout.lock_path, layout.data_root.join("runtime.lock"));
    }

    #[test]
    fn folders_with_spaces_are_ordinary() {
        let layout =
            PortableLayout::for_executable(Path::new("/media/USB DRIVE/ArcScan Portable/ArcScan"))
                .unwrap();
        assert_eq!(
            layout.database_path,
            Path::new("/media/USB DRIVE/ArcScan Portable/ArcScanData/arcscan.db")
        );
    }

    #[test]
    fn non_ascii_folders_are_ordinary() {
        // A technician's stick is as likely to be labelled in Japanese or
        // Norwegian as in English, and the path is never parsed as bytes.
        for root in [
            "/mnt/データ/ArcScan",
            "/mnt/Verktøy/ArcScan",
            "/mnt/Больница/ArcScan",
            "/mnt/ArcScan (copy #2)/ArcScan",
        ] {
            let exe = PathBuf::from(root).join("ArcScan");
            let layout = PortableLayout::for_executable(&exe).unwrap();
            assert_eq!(layout.portable_root, Path::new(root));
            assert_eq!(
                layout.database_path,
                Path::new(root).join("ArcScanData").join("arcscan.db")
            );
        }
    }

    #[test]
    fn deeply_nested_folders_are_ordinary() {
        let deep: PathBuf = (0..24).fold(PathBuf::from("/mnt"), |acc, i| acc.join(format!("d{i}")));
        let layout = PortableLayout::for_executable(&deep.join("ArcScan")).unwrap();
        assert_eq!(layout.portable_root, deep);
        assert!(layout.database_path.starts_with(&deep));
    }

    #[test]
    fn the_current_directory_is_never_consulted() {
        // The same executable path resolves to the same data root from any
        // working directory. This is the property a shortcut, a `cmd /k`, an
        // Explorer double-click and a scheduled task all differ on.
        let exe = Path::new("/media/stick/ArcScan Portable/ArcScan");
        let before = PortableLayout::for_executable(exe).unwrap();
        let original = std::env::current_dir().ok();
        std::env::set_current_dir("/").unwrap();
        let from_root = PortableLayout::for_executable(exe).unwrap();
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        let from_temp = PortableLayout::for_executable(exe).unwrap();
        if let Some(dir) = original {
            let _ = std::env::set_current_dir(dir);
        }
        assert_eq!(before, from_root);
        assert_eq!(before, from_temp);
    }

    #[test]
    fn two_portable_folders_share_nothing() {
        let a = PortableLayout::for_executable(Path::new("/mnt/a/ArcScan")).unwrap();
        let b = PortableLayout::for_executable(Path::new("/mnt/b/ArcScan")).unwrap();
        assert_ne!(a.data_root, b.data_root);
        assert_ne!(a.database_path, b.database_path);
        assert_ne!(a.webview_data_path, b.webview_data_path);
        assert_ne!(a.lock_path, b.lock_path);
        // Neither is inside the other, so neither can reach the other's data.
        assert!(!a.data_root.starts_with(&b.data_root));
        assert!(!b.data_root.starts_with(&a.data_root));
    }

    #[test]
    fn a_bare_filename_has_no_portable_root() {
        assert_eq!(
            PortableLayout::for_executable(Path::new("ArcScan")),
            Err(PortableError::NoExecutableDirectory)
        );
        assert_eq!(
            PortableLayout::for_executable(Path::new("")),
            Err(PortableError::NoExecutableDirectory)
        );
    }

    #[test]
    fn unc_paths_are_network_locations() {
        for path in [
            r"\\fileserver\tools\ArcScan",
            r"\\fileserver\tools",
            r"\\10.0.0.5\share\ArcScan Portable",
            r"\\?\UNC\fileserver\tools\ArcScan",
            r"\\?\unc\fileserver\tools",
            "//fileserver/tools/ArcScan",
        ] {
            assert_eq!(
                classify_location(Path::new(path), &NeverRemote),
                LocationKind::Network,
                "{path} should be refused"
            );
        }
    }

    #[test]
    fn verbatim_and_device_local_paths_are_not_network_locations() {
        // `\\?\C:\...` is a local path spelled verbatim, and `\\.\` is the
        // device namespace. Treating either as a share would refuse to run on
        // a perfectly ordinary local folder.
        for path in [r"\\?\C:\Tools\ArcScan", r"\\.\C:\Tools\ArcScan"] {
            assert_eq!(
                classify_location(Path::new(path), &NeverRemote),
                LocationKind::Local,
                "{path} should be allowed"
            );
        }
    }

    #[test]
    fn ordinary_local_paths_are_allowed() {
        for path in [
            r"C:\Program Files\ArcScan",
            r"E:\Tools\ArcScan",
            r"D:\ArcScan Portable",
            "/media/stick/ArcScan",
        ] {
            assert_eq!(
                classify_location(Path::new(path), &NeverRemote),
                LocationKind::Local,
                "{path} should be allowed"
            );
        }
    }

    #[test]
    fn a_mapped_network_drive_is_a_network_location() {
        // No UNC syntax to go on: a mapped drive looks exactly like a local one
        // and only the drive type tells them apart.
        assert_eq!(
            classify_location(Path::new(r"Z:\tools\ArcScan"), &AlwaysRemote),
            LocationKind::Network
        );
    }

    #[test]
    fn every_error_says_what_to_do_and_never_mentions_a_fallback() {
        let errors = [
            PortableError::NoExecutableDirectory,
            PortableError::NetworkLocation {
                location: r"\\srv\share".into(),
            },
            PortableError::CannotCreateDataDir {
                path: "E:/x".into(),
                detail: "access denied".into(),
            },
            PortableError::NotWritable {
                path: "E:/x".into(),
                detail: "read-only file system".into(),
            },
            PortableError::AlreadyRunning {
                path: "E:/x".into(),
            },
            PortableError::LockUnavailable {
                path: "E:/x".into(),
                detail: "access denied".into(),
            },
            PortableError::DatabaseUnavailable {
                path: "E:/x/arcscan.db".into(),
                detail: "disk I/O error".into(),
            },
        ];
        for error in errors {
            let message = error.message();
            assert!(message.contains("ArcScan Portable"), "{error:?}");
            // The one thing none of these may ever suggest.
            let lower = message.to_lowercase();
            for forbidden in ["appdata", "application data", "instead", "fell back"] {
                assert!(!lower.contains(forbidden), "{error:?} mentions {forbidden}");
            }
            // Display folds to one line for a log; the detail stays separate
            // from the sentence an operator reads first.
            assert!(!error.to_string().contains('\n'));
        }
    }

    #[test]
    fn the_installed_layout_is_the_one_every_release_has_had() {
        let paths = RuntimePaths::installed(PathBuf::from("/home/a/.local/share/com.arcscan.app"));
        assert_eq!(paths.edition, Edition::Installed);
        assert_eq!(
            paths.database_path,
            Path::new("/home/a/.local/share/com.arcscan.app/arcscan.db")
        );
        // No explicit WebView directory: the installed profile stays wherever
        // it already is, which is the whole point of not touching it.
        assert_eq!(paths.webview_data_path, None);
        assert_eq!(paths.portable_root, None);
    }

    #[test]
    fn the_installed_and_portable_data_roots_are_never_the_same_place() {
        let installed =
            RuntimePaths::installed(PathBuf::from("/home/a/.local/share/com.arcscan.app"));
        let layout =
            PortableLayout::for_executable(Path::new("/media/stick/ArcScan/ArcScan")).unwrap();
        let portable = RuntimePaths::portable(&layout);

        assert_ne!(installed.data_root, portable.data_root);
        assert_ne!(installed.database_path, portable.database_path);
        assert!(!portable.database_path.starts_with(&installed.data_root));
        assert!(!installed.database_path.starts_with(&portable.data_root));
        assert_eq!(
            portable.webview_data_path.as_deref(),
            Some(Path::new("/media/stick/ArcScan/ArcScanData/WebView"))
        );
    }

    #[test]
    fn runtime_info_carries_the_data_root_and_no_other_path() {
        let layout =
            PortableLayout::for_executable(Path::new("/media/stick/ArcScan/ArcScan")).unwrap();
        let info = RuntimePaths::portable(&layout).info();
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"data_root\""));
        assert!(json.contains("/media/stick/ArcScan/ArcScanData"));
        // The database and the WebView profile are resolved in Rust and stay
        // there: nothing the frontend receives lets it rebuild them.
        assert!(!json.contains("arcscan.db"));
        assert!(!json.contains("WebView"));
        assert!(!json.contains("runtime.lock"));

        // And the fields the interface actually renders are all present.
        for key in [
            "edition",
            "version",
            "platform",
            "architecture",
            "data_root",
            "writable",
            "updater_mode",
        ] {
            assert!(json.contains(&format!("\"{key}\"")), "missing {key}");
        }
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[cfg(windows)]
    #[test]
    fn drive_roots_come_out_as_getdrivetype_wants_them() {
        assert_eq!(drive_root(Path::new(r"E:\Tools")).as_deref(), Some("E:\\"));
        assert_eq!(drive_root(Path::new(r"c:\x")).as_deref(), Some("c:\\"));
        assert_eq!(drive_root(Path::new(r"\\srv\share")), None);
        assert_eq!(drive_root(Path::new("relative")), None);
    }
}
