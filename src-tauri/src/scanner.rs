//! The network scanner: liveness detection, port probing, MAC/vendor and
//! hostname resolution. Everything here is read-only discovery — no exploit,
//! brute-force, or credential logic exists or belongs in this module.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::ipparse;
use crate::oui;

/// Default TCP ports probed for the fallback liveness / service check.
pub const DEFAULT_PORTS: [u16; 6] = [22, 80, 443, 445, 3389, 8080];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub target: String,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub allow_public: bool,
    #[serde(default)]
    pub authorized: bool,
}

fn default_timeout() -> u64 {
    600
}
fn default_concurrency() -> usize {
    128
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResult {
    pub ip: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<u16>,
    pub response_ms: Option<u64>,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub target: String,
    pub duration_ms: u64,
    pub scanned: usize,
    pub hosts: Vec<HostResult>,
}

/// Build a `tokio::process::Command` that never pops a console window on
/// Windows. ArcScan is a GUI app, so any child process (`ping`, `arp`, the
/// launch helpers) must be spawned with CREATE_NO_WINDOW or a `/24` scan would
/// flash hundreds of `cmd` windows.
pub fn quiet_command(program: &str) -> tokio::process::Command {
    let std_cmd = std::process::Command::new(program);
    #[allow(unused_mut)]
    let mut std_cmd = std_cmd;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std_cmd.creation_flags(CREATE_NO_WINDOW);
    }
    tokio::process::Command::from(std_cmd)
}

/// Validate scan options against the safety policy, independently of the UI.
/// Returns the concrete host list on success.
pub fn validate(opts: &ScanOptions) -> Result<Vec<Ipv4Addr>, String> {
    if !opts.authorized {
        return Err(
            "Authorization not acknowledged. You must confirm you are authorized to scan this network."
                .into(),
        );
    }
    let hosts = ipparse::parse_target(&opts.target)?;
    if !opts.allow_public {
        if let Some(pub_ip) = hosts.iter().find(|ip| !ipparse::is_private(ip)) {
            return Err(format!(
                "`{pub_ip}` is a public address. ArcScan only scans private RFC1918 ranges unless the \"allow public range\" option is explicitly enabled."
            ));
        }
    }
    Ok(hosts)
}

/// Run a full scan. Applies safety validation first.
pub async fn run(opts: ScanOptions) -> Result<ScanResult, String> {
    let hosts = validate(&opts)?;

    let ports: Vec<u16> = if opts.ports.is_empty() {
        DEFAULT_PORTS.to_vec()
    } else {
        opts.ports.clone()
    };
    let timeout_ms = opts.timeout_ms.clamp(50, 10_000);
    let concurrency = opts.concurrency.clamp(1, 4096);
    let per_probe = Duration::from_millis(timeout_ms);

    let started = Instant::now();

    // Read the ARP cache once up front — a single OS call, not per host.
    let arp = read_arp_cache().await;

    // Probe liveness + ports concurrently, bounded by the semaphore.
    let sem = Arc::new(Semaphore::new(concurrency));
    let ports = Arc::new(ports);
    let scanned = hosts.len();

    let mut probe_results: Vec<(Ipv4Addr, Probe)> = stream::iter(hosts)
        .map(|ip| {
            let sem = sem.clone();
            let ports = ports.clone();
            async move {
                let _permit = sem.acquire().await.unwrap();
                let probe = probe_host(ip, &ports, per_probe).await;
                (ip, probe)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Keep only live hosts.
    probe_results.retain(|(_, p)| p.up);
    probe_results.sort_by_key(|(ip, _)| u32::from(*ip));

    // Resolve hostnames for the live hosts concurrently (bounded, short
    // per-lookup timeout) so N slow reverse-DNS misses collapse into one pass.
    let live_ips: Vec<Ipv4Addr> = probe_results.iter().map(|(ip, _)| *ip).collect();
    let dns_sem = Arc::new(Semaphore::new(256));
    let hostnames: HashMap<Ipv4Addr, String> = stream::iter(live_ips)
        .map(|ip| {
            let dns_sem = dns_sem.clone();
            async move {
                let _permit = dns_sem.acquire().await.unwrap();
                let name = resolve_hostname(ip).await;
                (ip, name)
            }
        })
        .buffer_unordered(256)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|(ip, name)| name.map(|n| (ip, n)))
        .collect();

    let now = chrono::Local::now().to_rfc3339();
    let hosts_out: Vec<HostResult> = probe_results
        .into_iter()
        .map(|(ip, probe)| {
            let mac = arp.get(&ip).cloned();
            let vendor = mac.as_deref().and_then(oui::lookup);
            HostResult {
                ip: ip.to_string(),
                hostname: hostnames.get(&ip).cloned(),
                mac,
                vendor,
                open_ports: probe.open_ports,
                response_ms: probe.response_ms,
                last_seen: now.clone(),
            }
        })
        .collect();

    Ok(ScanResult {
        target: opts.target,
        duration_ms: started.elapsed().as_millis() as u64,
        scanned,
        hosts: hosts_out,
    })
}

struct Probe {
    up: bool,
    open_ports: Vec<u16>,
    response_ms: Option<u64>,
}

async fn probe_host(ip: Ipv4Addr, ports: &[u16], per_probe: Duration) -> Probe {
    // Fire ICMP ping and all TCP probes concurrently.
    let ping_fut = icmp_ping(ip, per_probe);
    let tcp_fut = async {
        stream::iter(ports.iter().copied())
            .map(|port| async move { (port, tcp_probe(ip, port, per_probe).await) })
            .buffer_unordered(ports.len().max(1))
            .collect::<Vec<_>>()
            .await
    };

    let (ping_rtt, tcp_results) = futures::join!(ping_fut, tcp_fut);

    let mut open_ports = Vec::new();
    let mut best: Option<u64> = None;
    let mut alive_via_tcp = false;

    if let Some(rtt) = ping_rtt {
        best = Some(rtt.as_millis() as u64);
    }

    for (port, state) in tcp_results {
        match state {
            PortState::Open(d) => {
                open_ports.push(port);
                alive_via_tcp = true;
                let ms = d.as_millis() as u64;
                best = Some(best.map_or(ms, |b| b.min(ms)));
            }
            // A refused connection (RST) still proves the host is alive.
            PortState::Refused(d) => {
                alive_via_tcp = true;
                let ms = d.as_millis() as u64;
                best = Some(best.map_or(ms, |b| b.min(ms)));
            }
            PortState::NoReply => {}
        }
    }

    open_ports.sort_unstable();
    Probe {
        up: ping_rtt.is_some() || alive_via_tcp,
        open_ports,
        response_ms: best,
    }
}

enum PortState {
    Open(Duration),
    Refused(Duration),
    NoReply,
}

async fn tcp_probe(ip: Ipv4Addr, port: u16, per_probe: Duration) -> PortState {
    let addr = SocketAddr::new(IpAddr::V4(ip), port);
    let start = Instant::now();
    match timeout(per_probe, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => PortState::Open(start.elapsed()),
        Ok(Err(e)) => match e.kind() {
            // Actively refused == host is up but port closed.
            std::io::ErrorKind::ConnectionRefused => PortState::Refused(start.elapsed()),
            _ => PortState::NoReply,
        },
        Err(_) => PortState::NoReply, // timed out
    }
}

/// ICMP echo via the OS `ping` binary — deliberately avoids raw sockets so the
/// app never needs administrator/root privileges.
async fn icmp_ping(ip: Ipv4Addr, per_probe: Duration) -> Option<Duration> {
    let ms = per_probe.as_millis().max(1);
    let ip_s = ip.to_string();
    let start = Instant::now();

    let mut cmd = quiet_command("ping");
    #[cfg(windows)]
    {
        // -n 1 : one echo, -w <ms> : timeout in milliseconds
        cmd.args(["-n", "1", "-w", &ms.to_string(), &ip_s]);
    }
    #[cfg(target_os = "macos")]
    {
        // macOS: -c 1 count, -t <sec> total timeout (min 1s)
        let secs = ms.div_ceil(1000).max(1);
        cmd.args(["-c", "1", "-t", &secs.to_string(), &ip_s]);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Linux/BSD: -c 1 count, -W <sec> reply timeout, -n numeric
        let secs = ms.div_ceil(1000).max(1);
        cmd.args(["-c", "1", "-n", "-W", &secs.to_string(), &ip_s]);
    }

    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.stdin(std::process::Stdio::null());

    // Guard against a hung ping with a slightly larger outer timeout.
    let outer = per_probe + Duration::from_millis(500);
    match timeout(outer, cmd.status()).await {
        Ok(Ok(status)) if status.success() => Some(start.elapsed()),
        _ => None,
    }
}

/// Reverse-DNS a single address with a short timeout, run on a blocking thread
/// because `dns_lookup` is synchronous.
async fn resolve_hostname(ip: Ipv4Addr) -> Option<String> {
    let fut = tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&IpAddr::V4(ip)).ok());
    match timeout(Duration::from_millis(1500), fut).await {
        Ok(Ok(Some(name))) => {
            let name = name.trim().trim_end_matches('.').to_string();
            // Ignore results that just echo the IP back.
            if name.is_empty() || name == ip.to_string() {
                None
            } else {
                Some(name)
            }
        }
        _ => None,
    }
}

/// Read the system ARP cache in a single OS call and map IPv4 -> normalized MAC.
async fn read_arp_cache() -> HashMap<Ipv4Addr, String> {
    let mut cmd = quiet_command("arp");
    cmd.arg("-a");
    cmd.stdin(std::process::Stdio::null());
    let output = match timeout(Duration::from_secs(5), cmd.output()).await {
        Ok(Ok(o)) => o,
        _ => return HashMap::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_arp(&text)
}

/// Parse `arp -a` output across platforms. Windows uses `-` separators and a
/// column layout; unix uses `host (ip) at mac`. We just scan each line for an
/// IPv4 token and a MAC-looking token, which handles both.
fn parse_arp(text: &str) -> HashMap<Ipv4Addr, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let mut ip: Option<Ipv4Addr> = None;
        let mut mac: Option<String> = None;
        for raw in line.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
            let tok = raw.trim();
            if tok.is_empty() {
                continue;
            }
            if ip.is_none() {
                if let Ok(parsed) = tok.parse::<Ipv4Addr>() {
                    ip = Some(parsed);
                    continue;
                }
            }
            if mac.is_none() {
                if let Some(normalized) = normalize_mac(tok) {
                    mac = Some(normalized);
                }
            }
        }
        if let (Some(ip), Some(mac)) = (ip, mac) {
            map.insert(ip, mac);
        }
    }
    map
}

/// Normalize a MAC token (`00-11-22-33-44-55` or `00:11:22:...`) into an
/// uppercase colon-separated form. macOS `arp -a` prints unpadded octets
/// (`a0:ce:c8:d:cf:d1`), so 1-digit groups are zero-padded. Returns None for
/// non-MAC tokens (e.g. the `ff-ff-ff-ff-ff-ff` broadcast or malformed input).
fn normalize_mac(tok: &str) -> Option<String> {
    let sep = if tok.contains('-') {
        '-'
    } else if tok.contains(':') {
        ':'
    } else {
        return None;
    };
    let parts: Vec<&str> = tok.split(sep).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut octets = Vec::with_capacity(6);
    for p in parts {
        if p.is_empty() || p.len() > 2 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        octets.push(format!("{:0>2}", p.to_ascii_uppercase()));
    }
    let mac = octets.join(":");
    if mac == "FF:FF:FF:FF:FF:FF" || mac == "00:00:00:00:00:00" {
        return None;
    }
    Some(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_arp() {
        let sample = "\nInterface: 192.168.1.10 --- 0x5\n  Internet Address      Physical Address      Type\n  192.168.1.1           a0-11-22-33-44-55     dynamic\n  192.168.1.255         ff-ff-ff-ff-ff-ff     static\n";
        let map = parse_arp(sample);
        assert_eq!(
            map.get(&"192.168.1.1".parse().unwrap()).map(String::as_str),
            Some("A0:11:22:33:44:55")
        );
        // broadcast MAC is ignored
        assert!(!map.contains_key(&"192.168.1.255".parse().unwrap()));
    }

    #[test]
    fn parses_macos_unpadded_mac() {
        // macOS `arp -a` prints unpadded octets.
        let sample = "gateway.lan (10.0.1.1) at a0:ce:c8:d:cf:d1 on en0 ifscope [ethernet]\n";
        let map = parse_arp(sample);
        assert_eq!(
            map.get(&"10.0.1.1".parse().unwrap()).map(String::as_str),
            Some("A0:CE:C8:0D:CF:D1")
        );
    }

    #[test]
    fn parses_unix_arp() {
        let sample = "router.lan (192.168.0.1) at 3c:37:86:aa:bb:cc [ether] on eth0\n? (192.168.0.44) at 00:1a:2b:3c:4d:5e [ether] on eth0\n";
        let map = parse_arp(sample);
        assert_eq!(
            map.get(&"192.168.0.1".parse().unwrap()).map(String::as_str),
            Some("3C:37:86:AA:BB:CC")
        );
        assert_eq!(map.len(), 2);
    }
}
