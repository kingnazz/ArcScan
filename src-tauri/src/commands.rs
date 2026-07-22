//! Tauri command surface exposed to the frontend.

use std::net::Ipv4Addr;

use tauri::State;

use crate::db::{Db, ScanDetail, ScanSummary};
use crate::netinfo::{self, LocalNetwork};
use crate::scanner::{self, ScanOptions, ScanResult};

/// Reject anything that is not a bare, well-formed IPv4 address before it is
/// ever handed to a shell/launcher, to avoid argument injection.
fn validated_ipv4(ip: &str) -> Result<Ipv4Addr, String> {
    let ip = ip.trim();
    ip.parse::<Ipv4Addr>()
        .map_err(|_| format!("`{ip}` is not a valid IPv4 address."))
}

#[tauri::command]
pub async fn scan_network(window: tauri::Window, opts: ScanOptions) -> Result<ScanResult, String> {
    use tauri::Emitter;
    // Forward streamed scan progress to the UI as `scan:progress` events.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let win = window.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = win.emit("scan:progress", progress);
        }
    });
    scanner::run(opts, Some(tx)).await
}

#[tauri::command]
pub fn save_scan(db: State<'_, Db>, result: ScanResult) -> Result<i64, String> {
    db.save_scan(&result)
}

#[tauri::command]
pub fn list_scans(db: State<'_, Db>) -> Result<Vec<ScanSummary>, String> {
    db.list_scans()
}

#[tauri::command]
pub fn get_scan(db: State<'_, Db>, id: i64) -> Result<ScanDetail, String> {
    db.get_scan(id)
}

#[tauri::command]
pub fn delete_scan(db: State<'_, Db>, id: i64) -> Result<(), String> {
    db.delete_scan(id)
}

#[tauri::command]
pub fn last_scan_ips(db: State<'_, Db>) -> Result<Vec<String>, String> {
    db.last_scan_ips()
}

/// Detect the machine's own IPv4 networks so the UI can auto-fill the target
/// with the local subnet.
#[tauri::command]
pub fn detect_networks() -> Vec<LocalNetwork> {
    netinfo::detect()
}

/// Open the ArcScan releases page so the user can check for and download a
/// newer version.
#[tauri::command]
pub async fn open_releases(app: tauri::AppHandle) -> Result<(), String> {
    open_external(&app, "https://github.com/kingnazz/ArcScan/releases")
}

/// Write already-formatted export text (CSV/JSON/XML, built on the frontend) to
/// an operator-chosen path from the native save dialog.
#[tauri::command]
pub fn save_text(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write {path}: {e}"))
}

/// Send a Wake-on-LAN magic packet to the given MAC address (broadcast UDP).
#[tauri::command]
pub fn wake_on_lan(mac: String) -> Result<(), String> {
    let bytes = parse_mac(&mac)?;
    // Magic packet: 6 x 0xFF followed by the MAC repeated 16 times.
    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&bytes);
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("Failed to open socket: {e}"))?;
    socket
        .set_broadcast(true)
        .map_err(|e| format!("Failed to enable broadcast: {e}"))?;
    socket
        .send_to(&packet, "255.255.255.255:9")
        .map_err(|e| format!("Failed to send magic packet: {e}"))?;
    Ok(())
}

/// Parse a MAC address in any common separator style into 6 bytes.
fn parse_mac(mac: &str) -> Result<[u8; 6], String> {
    let hex: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return Err(format!("`{mac}` is not a valid MAC address."));
    }
    let mut out = [0u8; 6];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("`{mac}` is not a valid MAC address."))?;
    }
    Ok(out)
}

/// Open the host's SMB/Windows file shares in the system file browser.
#[tauri::command]
pub async fn open_smb(app: tauri::AppHandle, ip: String) -> Result<(), String> {
    let ip = validated_ipv4(&ip)?;
    #[cfg(windows)]
    {
        let _ = &app;
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(format!("\\\\{ip}"));
        cmd.spawn()
            .map_err(|e| format!("Failed to open shares: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // macOS Finder and Linux file managers understand smb:// URLs.
        open_external(&app, &format!("smb://{ip}"))
    }
}

#[tauri::command]
pub async fn open_web(app: tauri::AppHandle, ip: String, port: Option<u16>) -> Result<(), String> {
    let ip = validated_ipv4(&ip)?;
    let scheme = match port {
        Some(443) | Some(8443) => "https",
        _ => "http",
    };
    let url = match port {
        Some(p) if p != 80 && p != 443 => format!("{scheme}://{ip}:{p}"),
        _ => format!("{scheme}://{ip}"),
    };
    open_external(&app, &url)
}

#[tauri::command]
pub async fn open_rdp(ip: String) -> Result<(), String> {
    let ip = validated_ipv4(&ip)?;
    #[cfg(windows)]
    {
        // mstsc /v:<ip> — spawned without a console window.
        let mut cmd = scanner::quiet_command("mstsc");
        cmd.arg(format!("/v:{ip}"));
        cmd.spawn()
            .map_err(|e| format!("Failed to launch RDP client: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // Microsoft Remote Desktop / Windows App registers the rdp:// scheme.
        let mut cmd = std::process::Command::new("open");
        cmd.arg(format!("rdp://full%20address=s:{ip}"));
        cmd.spawn().map_err(|e| {
            format!("Failed to launch RDP client (is Windows App / Microsoft Remote Desktop installed?): {e}")
        })?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Best-effort on Linux: try common RDP clients.
        for (client, args) in [
            ("xfreerdp", vec![format!("/v:{ip}")]),
            ("remmina", vec!["-c".into(), format!("rdp://{ip}")]),
        ] {
            let mut cmd = std::process::Command::new(client);
            cmd.args(&args);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        Err(format!("No RDP client found to connect to {ip}."))
    }
}

#[tauri::command]
pub async fn open_ssh(ip: String) -> Result<(), String> {
    let ip = validated_ipv4(&ip)?;
    // SSH is the intentional exception: it should open a visible terminal for
    // the operator to interact with, so we do NOT suppress the window.
    #[cfg(windows)]
    {
        // Launch an interactive ssh session in a new console window.
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "start", "ssh", &ip.to_string()]);
        cmd.spawn()
            .map_err(|e| format!("Failed to launch SSH: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // Open Terminal and run an interactive ssh session. `ip` is a validated
        // bare IPv4 (digits and dots only), so interpolating it into the
        // AppleScript string cannot inject additional commands.
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"ssh {ip}\"\nend tell"
        );
        let mut cmd = std::process::Command::new("osascript");
        cmd.args(["-e", &script]);
        cmd.spawn().map_err(|e| format!("Failed to launch SSH: {e}"))?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Best-effort on Linux: try a few common terminal emulators.
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let mut cmd = std::process::Command::new(term);
            cmd.args(["-e", "ssh", &ip.to_string()]);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        Err("No terminal emulator found to launch SSH.".into())
    }
}

fn open_external(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("Failed to open {url}: {e}"))
}
