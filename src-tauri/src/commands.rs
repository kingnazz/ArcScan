//! Tauri command surface exposed to the frontend.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use tauri::State;

use serde::Serialize;

use crate::db::{
    BulkOutcome, ChangeFeed, Db, DeviceDetail, InventorySummary, NetworkScope, SavedScan,
    ScanDetail, ScanSummary,
};
use crate::inventory::{ChangeState, Device, DeviceStatus, ScanComparison};
use crate::netinfo::{self, LocalNetwork};
use crate::ports;
use crate::scanner::{self, ScanEvent, ScanOptions, ScanResult};

/// Reject anything that is not a bare, well-formed IPv4 address before it is
/// ever handed to a shell/launcher, to avoid argument injection.
fn validated_ipv4(ip: &str) -> Result<Ipv4Addr, String> {
    let ip = ip.trim();
    ip.parse::<Ipv4Addr>()
        .map_err(|_| format!("`{ip}` is not a valid IPv4 address."))
}

/// What a scan would do, so the UI can show the workload and any warning before
/// the operator commits to it.
#[derive(Debug, Clone, Serialize)]
pub struct ScanPreview {
    pub total: usize,
    pub port_count: usize,
    pub workload: u64,
    pub warning: Option<String>,
}

/// Validate a scan request without running it. The same code path the scan
/// itself uses, so the preview can never disagree with what happens next.
#[tauri::command]
pub fn preview_scan(opts: ScanOptions) -> Result<ScanPreview, String> {
    let plan = scanner::plan(&opts)?;
    Ok(ScanPreview {
        total: plan.hosts.len(),
        port_count: plan.ports.len(),
        workload: plan.workload,
        warning: plan.warning,
    })
}

/// Parse a human-written port specification. The backend is the source of truth
/// for ports, so the UI asks it rather than reimplementing the rules.
#[tauri::command]
pub fn parse_port_spec(spec: String) -> Result<Vec<u16>, String> {
    ports::parse_spec(&spec)
}

/// One known TCP service.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceInfo {
    pub port: u16,
    pub name: String,
    /// True for remote-control and file-sharing surfaces the UI marks as worth
    /// a second look.
    pub sensitive: bool,
}

/// One device type ArcScan is prepared to name, with the word it shows.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceTypeInfo {
    /// The stored value, e.g. `media_device`.
    pub id: String,
    /// The word a person reads, e.g. `Media device`.
    pub label: String,
}

/// The device-type vocabulary, fetched once at startup for the Inventory's type
/// filter. Same reasoning as [`service_catalog`]: the backend decides what types
/// exist, so the interface asks rather than keeping a list that can drift.
#[tauri::command]
pub fn device_type_catalog() -> Vec<DeviceTypeInfo> {
    crate::discovery::DeviceType::ALL
        .iter()
        .map(|kind| DeviceTypeInfo {
            id: kind.as_str().to_string(),
            label: kind.label().to_string(),
        })
        .collect()
}

/// The service-name table, fetched once at startup so the UI does not keep a
/// second copy of it that can drift out of sync with the scanner.
#[tauri::command]
pub fn service_catalog() -> Vec<ServiceInfo> {
    ports::catalog()
        .into_iter()
        .map(|(port, name, sensitive)| ServiceInfo {
            port,
            name: name.to_string(),
            sensitive,
        })
        .collect()
}

/// Run a scan, streaming structured events to the window as they happen so
/// devices appear while the scan is still running.
///
/// Every event carries the scan id, and the UI drops events whose id is not the
/// scan it is currently showing. That is what keeps a slow event from a
/// previous scan out of the current one's table.
#[tauri::command]
pub async fn scan_network(window: tauri::Window, opts: ScanOptions) -> Result<ScanResult, String> {
    use tauri::Emitter;

    let scan_id = scanner::next_scan_id();
    // Bounded: if the bridge cannot keep up, the scanner waits for capacity
    // (dropping only advisory progress events), so event memory cannot grow
    // without limit during a wide scan.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ScanEvent>(scanner::EVENT_CHANNEL_CAPACITY);
    let win = window.clone();
    let bridge = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = match event {
                ScanEvent::Started(payload) => win.emit("scan:started", payload),
                ScanEvent::Progress(payload) => win.emit("scan:progress", payload),
                ScanEvent::HostDiscovered { scan_id, host } => {
                    win.emit("scan:host-discovered", HostEvent { scan_id, host })
                }
                ScanEvent::HostUpdated { scan_id, host } => {
                    win.emit("scan:host-updated", HostEvent { scan_id, host })
                }
                ScanEvent::HostRemoved { scan_id, ip } => {
                    win.emit("scan:host-removed", RemovedEvent { scan_id, ip })
                }
            };
        }
    });

    let result = scanner::run(opts, scan_id, Some(tx)).await;
    // Draining the bridge before returning guarantees the UI has every host
    // event in hand by the time the promise resolves, so the streamed table and
    // the returned result cannot disagree.
    let _ = bridge.await;

    match &result {
        Ok(scan) if scan.cancelled => {
            let _ = window.emit("scan:cancelled", scan);
        }
        Ok(scan) => {
            let _ = window.emit("scan:completed", scan);
        }
        Err(_) => {}
    }
    result
}

#[derive(Serialize, Clone)]
struct HostEvent {
    scan_id: u64,
    host: Box<crate::scanner::HostResult>,
}

#[derive(Serialize, Clone)]
struct RemovedEvent {
    scan_id: u64,
    ip: String,
}

/// Ask the in-flight scan to stop. It finishes early and still returns whatever
/// hosts were discovered up to that point.
#[tauri::command]
pub fn cancel_scan() {
    scanner::request_cancel();
}

/// Save a scan and return its change summary in the same call, so the UI never
/// has to ask twice for the state it shows right after a scan finishes.
#[tauri::command]
pub fn save_scan(db: State<'_, Db>, result: ScanResult) -> Result<SavedScan, String> {
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

/// Compare a saved scan with the most recent earlier scan of the same target and
/// profile.
#[tauri::command]
pub fn compare_scan(db: State<'_, Db>, id: i64) -> Result<ScanComparison, String> {
    db.compare_scan(id)
}

#[tauri::command]
pub fn delete_scan(db: State<'_, Db>, id: i64) -> Result<(), String> {
    db.delete_scan(id)
}

/// Apply the history retention setting, keeping the newest `keep` scans.
#[tauri::command]
pub fn prune_history(db: State<'_, Db>, keep: i64) -> Result<usize, String> {
    db.prune_history(keep)
}

#[tauri::command]
pub fn list_devices(db: State<'_, Db>) -> Result<Vec<Device>, String> {
    db.list_devices()
}

/// The persistent Inventory: every device ArcScan has ever recorded, with the
/// presence its network's latest completed scan supports.
#[tauri::command]
pub fn inventory_summary(db: State<'_, Db>) -> Result<InventorySummary, String> {
    db.inventory()
}

/// The Changes inbox, newest first, with the unreviewed count.
#[tauri::command]
pub fn list_change_events(db: State<'_, Db>) -> Result<ChangeFeed, String> {
    db.change_events()
}

/// Acknowledge, ignore or reopen change events. Rejects an unknown state rather
/// than writing it, so a typo cannot make events invisible to every filter.
#[tauri::command]
pub fn set_change_state(
    db: State<'_, Db>,
    ids: Vec<i64>,
    state: String,
) -> Result<BulkOutcome, String> {
    let state = ChangeState::parse(&state)
        .ok_or_else(|| format!("`{state}` is not a review state ArcScan recognises."))?;
    db.set_change_state(&ids, state)
}

/// Classify several devices at once, for the Inventory's bulk actions.
#[tauri::command]
pub fn set_device_statuses(
    db: State<'_, Db>,
    ids: Vec<i64>,
    status: String,
) -> Result<BulkOutcome, String> {
    db.set_device_statuses(&ids, DeviceStatus::parse(&status))
}

/// Every known network scope, for display and naming.
#[tauri::command]
pub fn list_network_scopes(db: State<'_, Db>) -> Result<Vec<NetworkScope>, String> {
    db.list_network_scopes()
}

/// Give a network scope an operator-chosen name, e.g. `Office LAN`.
#[tauri::command]
pub fn rename_network_scope(db: State<'_, Db>, id: i64, name: String) -> Result<(), String> {
    db.rename_network_scope(id, name)
}

#[tauri::command]
pub fn device_detail(db: State<'_, Db>, id: i64) -> Result<DeviceDetail, String> {
    db.device_detail(id)
}

#[tauri::command]
pub fn set_device_name(db: State<'_, Db>, id: i64, name: Option<String>) -> Result<(), String> {
    db.set_device_name(id, name)
}

#[tauri::command]
pub fn set_device_status(db: State<'_, Db>, id: i64, status: String) -> Result<(), String> {
    db.set_device_status(id, DeviceStatus::parse(&status))
}

#[tauri::command]
pub fn set_device_notes(db: State<'_, Db>, id: i64, notes: Option<String>) -> Result<(), String> {
    db.set_device_notes(id, notes)
}

/// Correct, change or clear ArcScan's detected device type for one device.
///
/// `device_type` is `None` for Auto and one of the shipped type words
/// otherwise. Anything else is refused with a message rather than stored: the
/// value crosses the boundary from the webview into the database, and an
/// explicit choice of Unknown is a real answer that a typo must not be able to
/// impersonate.
///
/// This changes what ArcScan *calls* the device and nothing else. Identity,
/// network scope, presence, trust status, name and notes are all untouched, and
/// no change event is recorded, because an operator correcting a label is not
/// something that happened on the network.
#[tauri::command]
pub fn set_device_type_override(
    db: State<'_, Db>,
    id: i64,
    device_type: Option<String>,
) -> Result<(), String> {
    db.set_device_type_override(id, device_type)
}

/// Build the redacted discovery report for one device, for the clipboard.
///
/// Returns a string. It contacts nothing, writes no file and records no
/// telemetry; the caller copies it. What it deliberately omits is documented on
/// [`crate::discovery::diagnostics`] and enforced there.
#[tauri::command]
pub fn device_discovery_report(db: State<'_, Db>, id: i64) -> Result<String, String> {
    db.device_discovery_report(id, env!("CARGO_PKG_VERSION"))
}

/// Note bodies for the devices an export covers.
#[tauri::command]
pub fn device_notes(db: State<'_, Db>, ids: Vec<i64>) -> Result<Vec<(i64, String)>, String> {
    db.device_notes(&ids)
}

/// One-time adoption of the device labels v1.6 kept in browser local storage, so
/// upgrading to v1.7 does not lose the names an operator already gave devices.
#[tauri::command]
pub fn import_device_labels(
    db: State<'_, Db>,
    labels: HashMap<String, String>,
) -> Result<usize, String> {
    db.import_device_labels(labels)
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

/// Open the privacy notes in the system browser. A fixed trusted destination —
/// the frontend cannot pass a URL, so this surface cannot be redirected.
#[tauri::command]
pub async fn open_privacy(app: tauri::AppHandle) -> Result<(), String> {
    open_external(&app, "https://kingnazz.github.io/ArcScan/privacy.html")
}

/// Extensions an export may be written under, matching the formats the UI
/// offers. Everything else is refused, so this command cannot be used to drop
/// arbitrary file types even from a compromised webview.
const EXPORT_EXTENSIONS: [&str; 3] = ["csv", "json", "xml"];

/// Largest export accepted, as a plain sanity bound. A full /16 of devices is
/// far below this; anything larger is not an export ArcScan produced.
const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;

fn validate_export_path(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err("Exports can only be written to an absolute path from the save dialog.".into());
    }
    let ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EXPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    if !ok {
        return Err("Exports can only be saved as .csv, .json or .xml files.".into());
    }
    Ok(())
}

/// Write already-formatted export text (CSV/JSON/XML, built on the frontend) to
/// an operator-chosen path from the native save dialog. The path is validated
/// to look like an export destination before anything touches the disk.
#[tauri::command]
pub fn save_text(path: String, contents: String) -> Result<(), String> {
    validate_export_path(&path)?;
    if contents.len() > MAX_EXPORT_BYTES {
        return Err("This export is unreasonably large and was not written.".into());
    }
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

/// Build the URL the Web action opens: a validated bare IPv4 plus an optional
/// port, so nothing the frontend sends can smuggle a different host or scheme.
fn web_url(ip: Ipv4Addr, port: Option<u16>) -> String {
    let scheme = match port {
        Some(443) | Some(8443) => "https",
        _ => "http",
    };
    match port {
        Some(p) if p != 80 && p != 443 => format!("{scheme}://{ip}:{p}"),
        _ => format!("{scheme}://{ip}"),
    }
}

#[tauri::command]
pub async fn open_web(app: tauri::AppHandle, ip: String, port: Option<u16>) -> Result<(), String> {
    let ip = validated_ipv4(&ip)?;
    open_external(&app, &web_url(ip, port))
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
        let script =
            format!("tell application \"Terminal\"\nactivate\ndo script \"ssh {ip}\"\nend tell");
        let mut cmd = std::process::Command::new("osascript");
        cmd.args(["-e", &script]);
        cmd.spawn()
            .map_err(|e| format!("Failed to launch SSH: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_everything_that_is_not_a_bare_ipv4() {
        // Anything that could smuggle arguments, hosts or schemes into a
        // launcher must be refused before it reaches one.
        for bad in [
            "",
            "example.com",
            "10.0.0.1; rm -rf /",
            "10.0.0.1 --flag",
            "10.0.0.1/24",
            "10.0.0.1:8080",
            "::1",
            "fe80::1",
            "10.0.0",
            "10.0.0.256",
            "http://10.0.0.1",
            "10.0.0.1\nevil",
            "$(reboot)",
            "10.0.0.1&calc",
            "\"10.0.0.1\"",
        ] {
            assert!(validated_ipv4(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn accepts_ordinary_addresses_with_surrounding_whitespace() {
        assert_eq!(
            validated_ipv4(" 192.168.1.20 ").unwrap(),
            "192.168.1.20".parse::<Ipv4Addr>().unwrap()
        );
    }

    #[test]
    fn web_urls_never_carry_anything_but_the_validated_address() {
        let ip: Ipv4Addr = "10.0.0.9".parse().unwrap();
        assert_eq!(web_url(ip, None), "http://10.0.0.9");
        assert_eq!(web_url(ip, Some(80)), "http://10.0.0.9");
        assert_eq!(web_url(ip, Some(443)), "https://10.0.0.9");
        assert_eq!(web_url(ip, Some(8443)), "https://10.0.0.9:8443");
        assert_eq!(web_url(ip, Some(3000)), "http://10.0.0.9:3000");
    }

    #[test]
    fn rejects_malformed_macs_for_wake_on_lan() {
        for bad in [
            "",
            "not-a-mac",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:FF:00",
            "zz:zz:zz:zz:zz:zz",
        ] {
            assert!(parse_mac(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(
            parse_mac("aa-bb-cc-00-11-22").unwrap(),
            [0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22]
        );
    }

    #[test]
    fn export_paths_are_restricted_to_export_files() {
        let root = if cfg!(windows) {
            "C:\\exports\\"
        } else {
            "/exports/"
        };
        assert!(validate_export_path(&format!("{root}scan.csv")).is_ok());
        assert!(validate_export_path(&format!("{root}scan.JSON")).is_ok());
        assert!(validate_export_path(&format!("{root}scan.xml")).is_ok());

        // Relative paths and non-export file types are refused.
        assert!(validate_export_path("scan.csv").is_err());
        assert!(validate_export_path(&format!("{root}scan")).is_err());
        assert!(validate_export_path(&format!("{root}scan.exe")).is_err());
        assert!(validate_export_path(&format!("{root}scan.sh")).is_err());
        assert!(validate_export_path(&format!("{root}.bashrc")).is_err());
    }
}
