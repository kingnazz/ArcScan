//! Tauri command handlers exposed to the frontend.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db;
use crate::scanner::{self, Host, ScanEvent, ScanOptions, ScanResult};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub cancel: Arc<AtomicBool>,
    pub scanning: AtomicBool,
}

impl AppState {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Mutex::new(conn),
            cancel: Arc::new(AtomicBool::new(false)),
            scanning: AtomicBool::new(false),
        }
    }
}

#[tauri::command]
pub async fn scan_network(
    app: AppHandle,
    state: State<'_, AppState>,
    options: ScanOptions,
) -> Result<ScanResult, String> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("A scan is already in progress.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);

    // Snapshot the previous scan's hosts to flag new devices. The lock is
    // released before any `.await` so the future stays `Send`.
    let previous_ips = {
        let conn = state.db.lock().map_err(|_| "Database lock poisoned.")?;
        db::last_scan_ips(&conn).unwrap_or_default()
    };

    let cancel = state.cancel.clone();
    let emit_app = app.clone();
    let on_event = move |event: ScanEvent| match event {
        ScanEvent::Progress(p) => {
            let _ = emit_app.emit("scan://progress", p);
        }
        ScanEvent::Host(h) => {
            let _ = emit_app.emit("scan://host", h);
        }
    };

    let scan_outcome = scanner::run_scan(&options, &previous_ips, cancel, on_event).await;

    state.scanning.store(false, Ordering::SeqCst);
    let mut result = scan_outcome?;

    // Persist the finished scan.
    {
        let mut conn = state.db.lock().map_err(|_| "Database lock poisoned.")?;
        match db::save_scan(&mut conn, &result) {
            Ok(id) => result.scan_id = Some(id),
            Err(e) => eprintln!("ArcScan: failed to persist scan: {e}"),
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn list_scans(state: State<'_, AppState>) -> Result<Vec<db::ScanSummary>, String> {
    let conn = state.db.lock().map_err(|_| "Database lock poisoned.")?;
    db::list_scans(&conn)
}

#[tauri::command]
pub fn get_scan_hosts(state: State<'_, AppState>, scan_id: i64) -> Result<Vec<Host>, String> {
    let conn = state.db.lock().map_err(|_| "Database lock poisoned.")?;
    db::get_scan_hosts(&conn, scan_id)
}

#[tauri::command]
pub fn delete_scan(state: State<'_, AppState>, scan_id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "Database lock poisoned.")?;
    db::delete_scan(&conn, scan_id)
}

/// Launch an external client for a discovered host. Read-only convenience —
/// it never sends credentials; it simply opens the appropriate local tool.
#[tauri::command]
pub fn launch_action(kind: String, ip: String, port: Option<u16>) -> Result<(), String> {
    // Reject anything that isn't a bare IPv4 address to avoid argument injection.
    let addr: std::net::Ipv4Addr = ip
        .parse()
        .map_err(|_| "Invalid IP address.".to_string())?;
    let ip = addr.to_string();

    match kind.as_str() {
        "web" => {
            let p = port.unwrap_or(80);
            let scheme = if p == 443 || p == 8443 { "https" } else { "http" };
            let url = if p == 80 || p == 443 {
                format!("{scheme}://{ip}")
            } else {
                format!("{scheme}://{ip}:{p}")
            };
            open_url(&url)
        }
        "rdp" => open_rdp(&ip, port.unwrap_or(3389)),
        "ssh" => open_ssh(&ip, port.unwrap_or(22)),
        other => Err(format!("Unknown action: {other}")),
    }
}

#[tauri::command]
pub fn write_text_file(path: String, contents: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write file: {e}"))
}

// ---- External launch helpers ----------------------------------------------

fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch {program}: {e}"))
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // `start` is a cmd builtin; the empty title arg avoids quoting issues.
        return spawn_detached("cmd", &["/C", "start", "", url]);
    }
    #[cfg(target_os = "macos")]
    {
        return spawn_detached("open", &[url]);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return spawn_detached("xdg-open", &[url]);
    }
    #[allow(unreachable_code)]
    Err("Opening URLs is not supported on this platform.".into())
}

fn open_rdp(ip: &str, port: u16) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let target = format!("/v:{ip}:{port}");
        return spawn_detached("mstsc", &[&target]);
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Try FreeRDP if it is installed; otherwise report gracefully.
        let target = format!("/v:{ip}:{port}");
        spawn_detached("xfreerdp", &[&target]).map_err(|_| {
            "No RDP client found. Install FreeRDP (xfreerdp) or use the Windows build.".to_string()
        })
    }
}

fn open_ssh(ip: &str, port: u16) -> Result<(), String> {
    let port_s = port.to_string();
    #[cfg(target_os = "windows")]
    {
        // Open ssh inside a new console window.
        return spawn_detached(
            "cmd",
            &["/C", "start", "cmd", "/K", "ssh", "-p", &port_s, ip],
        );
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell app \"Terminal\" to do script \"ssh -p {port_s} {ip}\"");
        return spawn_detached("osascript", &["-e", &script]);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Best-effort terminal launch; falls back to the generic alias.
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            if spawn_detached(term, &["-e", &format!("ssh -p {port_s} {ip}")]).is_ok() {
                return Ok(());
            }
        }
        return Err("No terminal emulator found to launch SSH.".into());
    }
    #[allow(unreachable_code)]
    Err("SSH launch is not supported on this platform.".into())
}

/// Resolve the on-disk path for the scan-history database.
pub fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Cannot resolve app data dir: {e}"))?;
    Ok(dir.join("arcscan.db"))
}
