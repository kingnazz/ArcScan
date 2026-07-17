//! Tauri command surface exposed to the frontend.

use std::net::Ipv4Addr;

use tauri::State;

use crate::db::{Db, ScanDetail, ScanSummary};
use crate::scanner::{self, HostResult, ScanOptions, ScanResult};

/// Reject anything that is not a bare, well-formed IPv4 address before it is
/// ever handed to a shell/launcher, to avoid argument injection.
fn validated_ipv4(ip: &str) -> Result<Ipv4Addr, String> {
    let ip = ip.trim();
    ip.parse::<Ipv4Addr>()
        .map_err(|_| format!("`{ip}` is not a valid IPv4 address."))
}

#[tauri::command]
pub async fn scan_network(opts: ScanOptions) -> Result<ScanResult, String> {
    scanner::run(opts).await
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

/// Build the CSV text for a result set. Kept in Rust so exported files are
/// byte-identical regardless of how the export is triggered.
#[tauri::command]
pub fn build_csv(hosts: Vec<HostResult>) -> String {
    csv_for(&hosts)
}

/// Write CSV to an operator-chosen path (obtained via the native save dialog on
/// the frontend). Only used inside Tauri.
#[tauri::command]
pub fn export_csv(path: String, hosts: Vec<HostResult>) -> Result<(), String> {
    let csv = csv_for(&hosts);
    std::fs::write(&path, csv).map_err(|e| format!("Failed to write {path}: {e}"))
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_for(hosts: &[HostResult]) -> String {
    let mut out = String::from(
        "IP,Hostname,MAC,Vendor,Open Ports,Response (ms),Last Seen\n",
    );
    for h in hosts {
        let ports = h
            .open_ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let row = [
            csv_field(&h.ip),
            csv_field(h.hostname.as_deref().unwrap_or("")),
            csv_field(h.mac.as_deref().unwrap_or("")),
            csv_field(h.vendor.as_deref().unwrap_or("")),
            csv_field(&ports),
            csv_field(&h.response_ms.map(|v| v.to_string()).unwrap_or_default()),
            csv_field(&h.last_seen),
        ]
        .join(",");
        out.push_str(&row);
        out.push('\n');
    }
    out
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
