//! Startup: the portable preflight, the fatal-error report, and the one window.
//!
//! This module is the seam between "which edition am I" and the rest of the
//! application. Everything edition-specific about starting up happens here and
//! nowhere else, so there is a single place to read to find out where ArcScan's
//! data goes and what happens when it cannot go there.

use std::path::Path;

use tauri::{AppHandle, WebviewWindowBuilder};

use crate::portable::{self, PortableGuard};
use crate::runtime::{Edition, PortableError, PortableLayout, RuntimePaths, SystemDriveType};

/// A portable copy that has passed its preflight and holds its data root.
///
/// Kept in Tauri's application state for the life of the process: the lock
/// inside `_guard` is the claim on the folder, and dropping this releases it.
pub struct PortableStartup {
    pub layout: PortableLayout,
    _guard: PortableGuard,
}

/// Run whatever startup this edition needs before Tauri exists.
///
/// Installed builds need nothing here, and get `Ok(None)` — the same code path
/// they have always taken, with the application-data directory resolved later
/// from the app handle exactly as before.
///
/// Portable builds resolve their folder from `current_exe()`, prove it works,
/// and claim it. A failure here is fatal by design: the alternative is a
/// portable ArcScan quietly writing somewhere else.
pub fn portable_startup() -> Result<Option<PortableStartup>, PortableError> {
    if !Edition::current().is_portable() {
        return Ok(None);
    }

    let exe = std::env::current_exe().map_err(|_| PortableError::NoExecutableDirectory)?;
    let layout = PortableLayout::for_executable(&exe)?;
    let guard = portable::preflight(&layout, &SystemDriveType)?;

    Ok(Some(PortableStartup {
        layout,
        _guard: guard,
    }))
}

/// Report a fatal startup failure to the operator, with no window to put it in.
///
/// Two channels, and both are unconditional.
///
/// Standard error always gets the message. A GUI-subsystem process on Windows
/// has no console attached when it is double-clicked, so nobody sees this in
/// ordinary use -- but a program that fails should say why on stderr, and a
/// script or a shell that captured it then has something to read. That is what
/// makes the portable failure paths observable to the verification script rather
/// than only to a person.
///
/// Windows additionally gets a `MessageBoxW`: a synchronous system modal that
/// needs no event loop, no WebView and no window, which is the whole reason the
/// preflight runs before Tauri starts rather than inside its setup hook.
pub fn report_fatal(error: &PortableError) {
    let message = match error.detail() {
        Some(detail) => format!("{}\n\n{detail}", error.message()),
        None => error.message(),
    };

    eprintln!("{message}");

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
        };

        let wide = |text: &str| -> Vec<u16> {
            std::ffi::OsStr::new(text)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let body = wide(&message);
        let title = wide("ArcScan Portable");
        // SAFETY: both buffers are NUL-terminated and outlive the call, and a
        // null owner window is what a process with no window passes.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
            );
        }
    }
}

/// Build the main window.
///
/// Tauri would normally do this itself from `app.windows[0]`, and for the
/// installed edition this is precisely that: `from_config` with the same config
/// object, which is the identical call `tauri::app::setup` makes. `create` is
/// set to false in `tauri.conf.json` only so that the portable edition can add
/// one thing before the WebView is created — the profile directory it must use.
///
/// That ordering is the reason this exists. A WebView's data directory has to be
/// chosen when its environment is created; there is no supported way to move it
/// afterwards, and copying preferences into place after startup would be a
/// convincing-looking fake rather than isolation.
pub fn build_main_window(
    app: &AppHandle,
    paths: &RuntimePaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .ok_or("tauri.conf.json declares no window")?
        .clone();

    let mut builder = WebviewWindowBuilder::from_config(app, &config)?;

    if let Some(profile) = &paths.webview_data_path {
        builder = builder.data_directory(profile.clone());
    }

    builder.build()?;
    Ok(())
}

/// Open the running edition's data folder in the system file manager.
///
/// The path is not a parameter. It is the data root this process resolved at
/// startup, read back out of application state, so the command cannot be
/// pointed anywhere: a compromised webview invoking it a thousand times opens
/// the same folder a thousand times.
pub fn reveal_data_folder(app: &AppHandle, data_root: &Path) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(data_root.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Failed to open the data folder: {e}"))
}
