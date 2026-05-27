//! Network discovery engine.
//!
//! ArcScan performs **read-only** host discovery only. For each target address
//! it issues an ICMP echo (via the OS `ping` utility, so no elevated raw-socket
//! privileges are required) and, in parallel, attempts TCP connections to a
//! small set of well-known service ports. A host is considered "up" if it
//! answers ICMP, accepts a TCP connection, or actively refuses one (RST) — all
//! of which prove the host is online. No payloads, credentials, or exploit
//! traffic are ever sent.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use crate::oui;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortResult {
    pub port: u16,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub ip: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<PortResult>,
    pub response_ms: Option<u64>,
    pub status: String,
    pub last_seen: String,
    pub is_new: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub target: String,
    pub timeout_ms: u64,
    pub concurrency: usize,
    pub ports: Vec<u16>,
    pub allow_public: bool,
    pub authorized: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scanned: usize,
    pub total: usize,
    pub found: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub scan_id: Option<i64>,
    pub target: String,
    pub started_at: String,
    pub finished_at: String,
    pub hosts: Vec<Host>,
    pub total_scanned: usize,
}

pub fn service_name(port: u16) -> &'static str {
    match port {
        22 => "SSH",
        80 => "HTTP",
        443 => "HTTPS",
        445 => "SMB",
        3389 => "RDP",
        8080 => "HTTP-Alt",
        8443 => "HTTPS-Alt",
        21 => "FTP",
        23 => "Telnet",
        25 => "SMTP",
        53 => "DNS",
        3306 => "MySQL",
        5432 => "PostgreSQL",
        _ => "TCP",
    }
}

pub fn is_private(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// Expand a CIDR block, dashed range, or single address into a list of IPv4
/// addresses. Mirrors the parsing in the frontend's `src/lib/ip.ts`.
pub fn parse_target(input: &str) -> Result<Vec<Ipv4Addr>, String> {
    let target = input.trim();
    if target.is_empty() {
        return Err("Enter an IP range or CIDR.".into());
    }

    const MAX_HOSTS: u64 = 65_536;

    let (lo, hi): (u32, u32) = if let Some((addr, prefix_str)) = target.split_once('/') {
        let base: Ipv4Addr = addr
            .trim()
            .parse()
            .map_err(|_| "Invalid CIDR address.".to_string())?;
        let prefix: u32 = prefix_str
            .trim()
            .parse()
            .map_err(|_| "Invalid CIDR prefix.".to_string())?;
        if prefix > 32 {
            return Err("CIDR prefix must be between 0 and 32.".into());
        }
        let base = u32::from(base);
        let mask: u32 = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
        (base & mask, (base & mask) | !mask)
    } else if let Some((start_str, end_str)) = target.split_once('-') {
        let start: Ipv4Addr = start_str
            .trim()
            .parse()
            .map_err(|_| "Invalid start address.".to_string())?;
        let start = u32::from(start);
        let end: u32 = {
            let e = end_str.trim();
            if e.contains('.') {
                u32::from(
                    e.parse::<Ipv4Addr>()
                        .map_err(|_| "Invalid end address.".to_string())?,
                )
            } else {
                let last: u32 = e.parse().map_err(|_| "Invalid end octet.".to_string())?;
                if last > 255 {
                    return Err("End octet must be 0-255.".into());
                }
                (start & 0xffff_ff00) | last
            }
        };
        (start.min(end), start.max(end))
    } else {
        let single: Ipv4Addr = target
            .parse()
            .map_err(|_| "Invalid IP address.".to_string())?;
        let v = u32::from(single);
        (v, v)
    };

    let count = (hi as u64) - (lo as u64) + 1;
    if count > MAX_HOSTS {
        return Err(format!(
            "Range too large ({count} hosts). Limit is {MAX_HOSTS}."
        ));
    }

    Ok((lo..=hi).map(Ipv4Addr::from).collect())
}

struct ProbeOutcome {
    alive: bool,
    open_ports: Vec<PortResult>,
    response_ms: Option<u64>,
}

/// Probe a single host: ICMP echo plus parallel TCP connects to `ports`.
async fn probe_host(ip: Ipv4Addr, ports: &[u16], timeout: Duration) -> ProbeOutcome {
    // ICMP echo via the OS ping utility (no raw socket privileges needed).
    let icmp_rtt = icmp_ping(ip, timeout).await;

    // TCP connects, all ports in parallel.
    let mut tasks = FuturesUnordered::new();
    for &port in ports {
        tasks.push(async move {
            let addr = SocketAddr::new(IpAddr::V4(ip), port);
            let start = Instant::now();
            match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
                Ok(Ok(_stream)) => (port, true, true, Some(start.elapsed())),
                Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => {
                    // Refused proves the host is online even though the port is closed.
                    (port, false, true, Some(start.elapsed()))
                }
                _ => (port, false, false, None),
            }
        });
    }

    let mut open_ports = Vec::new();
    let mut tcp_alive = false;
    let mut tcp_rtt: Option<Duration> = None;
    while let Some((port, is_open, responded, rtt)) = tasks.next().await {
        if responded {
            tcp_alive = true;
            if let Some(r) = rtt {
                tcp_rtt = Some(tcp_rtt.map_or(r, |cur| cur.min(r)));
            }
        }
        if is_open {
            open_ports.push(PortResult {
                port,
                service: service_name(port).to_string(),
            });
        }
    }
    open_ports.sort_by_key(|p| p.port);

    let response_ms = icmp_rtt.or_else(|| tcp_rtt.map(|d| d.as_millis() as u64));
    ProbeOutcome {
        alive: icmp_rtt.is_some() || tcp_alive,
        open_ports,
        response_ms,
    }
}

/// Issue one ICMP echo via the platform `ping` binary; returns RTT in ms.
async fn icmp_ping(ip: Ipv4Addr, timeout: Duration) -> Option<u64> {
    use tokio::process::Command;

    let ip_str = ip.to_string();
    let timeout_ms = timeout.as_millis().max(1) as u64;

    let mut cmd;
    #[cfg(target_os = "windows")]
    {
        cmd = Command::new("ping");
        cmd.args(["-n", "1", "-w", &timeout_ms.to_string(), &ip_str]);
        // Suppress the console window Windows would otherwise spawn for each
        // child process (CREATE_NO_WINDOW) — without this a scan flashes a
        // separate cmd window per host.
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(target_os = "macos")]
    {
        cmd = Command::new("ping");
        // macOS expects -W in milliseconds and -t (TTL) is unrelated; use -W.
        cmd.args(["-c", "1", "-W", &timeout_ms.to_string(), &ip_str]);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let timeout_secs = ((timeout_ms as f64) / 1000.0).ceil().max(1.0) as u64;
        cmd = Command::new("ping");
        cmd.args(["-c", "1", "-W", &timeout_secs.to_string(), &ip_str]);
    }

    cmd.kill_on_drop(true);
    let output = cmd.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_ping_rtt(&text)
}

/// Extract round-trip time (ms) from `ping` stdout across platforms.
fn parse_ping_rtt(text: &str) -> Option<u64> {
    // Look for "time=1.23 ms", "time=1ms", or "time<1ms".
    let lower = text.to_lowercase();
    if let Some(idx) = lower.find("time=") {
        let rest = &lower[idx + 5..];
        let num: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        return num.parse::<f64>().ok().map(|v| v.round() as u64);
    }
    if lower.contains("time<") {
        return Some(0);
    }
    // Host answered but no parsable time; report 0 rather than nothing.
    if lower.contains("ttl=") || lower.contains("bytes from") {
        Some(0)
    } else {
        None
    }
}

/// Read the OS ARP cache into an `ip -> mac` map (colon-separated, lower-case).
/// MAC addresses are only available for hosts on the same L2 segment.
fn read_arp_table() -> HashMap<String, String> {
    let mut map = HashMap::new();

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/net/arp") {
            for line in content.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                // IP HW-type Flags HW-address Mask Device
                if cols.len() >= 4 {
                    let ip = cols[0].to_string();
                    let mac = cols[3].to_lowercase();
                    if mac != "00:00:00:00:00:00" && mac.contains(':') {
                        map.insert(ip, mac);
                    }
                }
            }
        }
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let mut cmd = std::process::Command::new("arp");
        cmd.arg("-a");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        if let Ok(output) = cmd.output() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some((ip, mac)) = parse_arp_line(line) {
                    map.insert(ip, mac);
                }
            }
        }
    }

    map
}

/// Parse a single `arp -a` line (Windows / macOS formats) into (ip, mac).
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn parse_arp_line(line: &str) -> Option<(String, String)> {
    let mut ip: Option<String> = None;
    let mut mac: Option<String> = None;
    for token in line.split_whitespace() {
        let t = token.trim_matches(|c| c == '(' || c == ')');
        if t.parse::<std::net::Ipv4Addr>().is_ok() {
            ip = Some(t.to_string());
        } else {
            let normalized = t.replace('-', ":").to_lowercase();
            let parts: Vec<&str> = normalized.split(':').collect();
            if parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok()) {
                mac = Some(normalized);
            }
        }
    }
    match (ip, mac) {
        (Some(ip), Some(mac)) if mac != "ff:ff:ff:ff:ff:ff" => Some((ip, mac)),
        _ => None,
    }
}

/// Reverse-DNS a single address (blocking call, run off the async runtime).
fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    dns_lookup::lookup_addr(&IpAddr::V4(ip))
        .ok()
        .filter(|name| !name.is_empty() && name.parse::<Ipv4Addr>().is_err())
}

/// Run a full scan, invoking `on_event` for each progress tick and discovered
/// host. `cancel` is polled to allow cooperative cancellation. `previous_ips`
/// is the set of hosts seen in the prior saved scan, used to flag new devices.
pub async fn run_scan<F>(
    options: &ScanOptions,
    previous_ips: &HashSet<String>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    on_event: F,
) -> Result<ScanResult, String>
where
    F: Fn(ScanEvent) + Send + Sync + 'static,
{
    if !options.authorized {
        return Err("Scan blocked: authorization was not acknowledged.".into());
    }

    let addrs = parse_target(&options.target)?;

    if !options.allow_public {
        if let Some(public) = addrs.iter().find(|ip| !is_private(**ip)) {
            return Err(format!(
                "Refusing to scan public address {public}. Enable 'Allow public range' to override (authorized use only)."
            ));
        }
    }

    let ports: Vec<u16> = if options.ports.is_empty() {
        vec![22, 80, 443, 445, 3389, 8080]
    } else {
        options.ports.clone()
    };
    let timeout = Duration::from_millis(options.timeout_ms.clamp(50, 10_000));
    let concurrency = options.concurrency.clamp(1, 1024);

    let started_at = Utc::now().to_rfc3339();
    let total = addrs.len();
    let scanned = Arc::new(AtomicUsize::new(0));
    let found = Arc::new(AtomicUsize::new(0));
    let on_event = Arc::new(on_event);

    let ports = Arc::new(ports);
    let mut in_flight = FuturesUnordered::new();
    let mut iter = addrs.into_iter();
    let mut hosts: Vec<Host> = Vec::new();

    // Seed the pipeline up to the concurrency limit.
    for _ in 0..concurrency {
        match iter.next() {
            Some(ip) => in_flight.push(spawn_probe(ip, ports.clone(), timeout)),
            None => break,
        }
    }

    while let Some((ip, outcome)) = in_flight.next().await {
        let done = scanned.fetch_add(1, Ordering::Relaxed) + 1;

        if outcome.alive {
            let found_n = found.fetch_add(1, Ordering::Relaxed) + 1;
            let host = Host {
                ip: ip.to_string(),
                hostname: None,
                mac: None,
                vendor: None,
                open_ports: outcome.open_ports,
                response_ms: outcome.response_ms,
                status: "up".into(),
                last_seen: Utc::now().to_rfc3339(),
                is_new: !previous_ips.contains(&ip.to_string()),
            };
            (on_event)(ScanEvent::Host(host.clone()));
            (on_event)(ScanEvent::Progress(ScanProgress {
                scanned: done,
                total,
                found: found_n,
            }));
            hosts.push(host);
        } else {
            (on_event)(ScanEvent::Progress(ScanProgress {
                scanned: done,
                total,
                found: found.load(Ordering::Relaxed),
            }));
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }

        if let Some(ip) = iter.next() {
            in_flight.push(spawn_probe(ip, ports.clone(), timeout));
        }
    }

    // Enrich live hosts with reverse-DNS hostnames and ARP/vendor data.
    let arp = tokio::task::spawn_blocking(read_arp_table)
        .await
        .unwrap_or_default();

    for host in hosts.iter_mut() {
        if let Some(mac) = arp.get(&host.ip) {
            host.vendor = oui::vendor_for_mac(mac);
            host.mac = Some(mac.clone());
        }
        if let Ok(ip) = host.ip.parse::<Ipv4Addr>() {
            host.hostname = tokio::task::spawn_blocking(move || reverse_dns(ip))
                .await
                .unwrap_or(None);
        }
    }

    hosts.sort_by_key(|h| h.ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(0));

    Ok(ScanResult {
        scan_id: None,
        target: options.target.clone(),
        started_at,
        finished_at: Utc::now().to_rfc3339(),
        total_scanned: scanned.load(Ordering::Relaxed),
        hosts,
    })
}

fn spawn_probe(
    ip: Ipv4Addr,
    ports: Arc<Vec<u16>>,
    timeout: Duration,
) -> impl std::future::Future<Output = (Ipv4Addr, ProbeOutcome)> {
    async move {
        let outcome = probe_host(ip, &ports, timeout).await;
        (ip, outcome)
    }
}

/// Events streamed to the frontend during a scan.
pub enum ScanEvent {
    Progress(ScanProgress),
    Host(Host),
}
