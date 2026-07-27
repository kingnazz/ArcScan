//! The network scanner: liveness detection, port probing, MAC/vendor and
//! hostname resolution. Everything here is read-only discovery — no exploit,
//! brute-force, or credential logic exists or belongs in this module.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::ipparse;
use crate::netinfo;
use crate::oui;

/// True if `ip` falls within any (network, mask) range.
fn ip_in_ranges(ip: Ipv4Addr, ranges: &[(u32, u32)]) -> bool {
    let v = u32::from(ip);
    ranges.iter().any(|(net, mask)| v & mask == *net)
}

/// Cooperative cancellation for the in-flight scan. Only one scan runs at a
/// time (the UI disables Scan while one is active), so a single global flag is
/// enough. A cancelled scan returns the hosts found so far rather than an
/// error, so the user keeps partial results.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// Ask the running scan to stop as soon as it can.
pub fn request_cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

fn cancelled() -> bool {
    CANCEL.load(Ordering::Relaxed)
}

/// Progress update streamed to the UI while a scan runs.
#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub done: usize,
    pub total: usize,
    /// "probing" while hosts are being checked, then "resolving", then "done".
    pub phase: String,
}

/// Default TCP ports probed for liveness and quick service detection. A curated
/// spread of the ports that matter most on a typical LAN — kept small enough to
/// stay fast, wide enough to fingerprint most hosts.
pub const DEFAULT_PORTS: [u16; 14] = [
    21, 22, 23, 53, 80, 110, 139, 143, 443, 445, 3389, 5900, 8080, 8443,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub target: String,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

fn default_timeout() -> u64 {
    900
}
fn default_concurrency() -> usize {
    // Deliberately conservative: consumer routers and access points rate-limit
    // and drop ARP replies when hit with too many simultaneous connects, which
    // makes hosts vanish from a single scan. A gentler fan-out resolves more of
    // them in one pass.
    64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResult {
    pub ip: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<u16>,
    pub response_ms: Option<u64>,
    /// TTL from the ICMP echo reply, when a ping succeeded.
    pub ttl: Option<u8>,
    /// Coarse OS guess derived from the TTL (Angry-IP-style fetcher).
    pub os_guess: Option<String>,
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

/// Parse and validate the scan target into a concrete host list. Rejects
/// malformed input and oversized ranges (see `ipparse::MAX_HOSTS`).
pub fn validate(opts: &ScanOptions) -> Result<Vec<Ipv4Addr>, String> {
    ipparse::parse_target(&opts.target)
}

/// Run a full scan. `progress`, when provided, receives streamed progress
/// updates the command layer forwards to the UI as events.
pub async fn run(
    opts: ScanOptions,
    progress: Option<UnboundedSender<ScanProgress>>,
) -> Result<ScanResult, String> {
    let hosts = validate(&opts)?;

    // Clear any cancellation left over from a previous scan.
    CANCEL.store(false, Ordering::Relaxed);

    let ports: Vec<u16> = if opts.ports.is_empty() {
        DEFAULT_PORTS.to_vec()
    } else {
        opts.ports.clone()
    };
    let timeout_ms = opts.timeout_ms.clamp(50, 10_000);
    let concurrency = opts.concurrency.clamp(1, 4096);
    let per_probe = Duration::from_millis(timeout_ms);

    let started = Instant::now();

    // Probe liveness + ports concurrently, bounded by the semaphore.
    let sem = Arc::new(Semaphore::new(concurrency));
    let ports = Arc::new(ports);
    let scanned = hosts.len();

    // Progress: count completed probes and stream throttled updates to the UI.
    let done = Arc::new(AtomicUsize::new(0));
    let step = (scanned / 100).max(1);
    let emit = |p: ScanProgress| {
        if let Some(tx) = &progress {
            let _ = tx.send(p);
        }
    };
    emit(ScanProgress {
        done: 0,
        total: scanned,
        phase: "probing".into(),
    });

    let mut probe_results: Vec<(Ipv4Addr, Probe)> = stream::iter(hosts)
        .map(|ip| {
            let sem = sem.clone();
            let ports = ports.clone();
            let done = done.clone();
            let progress = progress.clone();
            async move {
                let _permit = sem.acquire().await.unwrap();
                // Once cancelled, drain the remaining hosts without probing so
                // the stream finishes immediately instead of running to the end.
                let probe = if cancelled() {
                    Probe {
                        up: false,
                        open_ports: Vec::new(),
                        response_ms: None,
                        ttl: None,
                    }
                } else {
                    probe_host(ip, &ports, per_probe).await
                };
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(tx) = &progress {
                    if d == scanned || d % step == 0 {
                        let _ = tx.send(ScanProgress {
                            done: d,
                            total: scanned,
                            phase: "probing".into(),
                        });
                    }
                }
                (ip, probe)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Let late ARP replies from slow devices settle before we read the cache, so
    // a single scan captures them instead of trickling across repeated scans.
    emit(ScanProgress {
        done: scanned,
        total: scanned,
        phase: "resolving".into(),
    });
    // A cancelled scan skips the settle so it stops promptly; the ARP cache is
    // still read below so whatever was found keeps its MAC and vendor.
    if !cancelled() {
        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    // Detect our own local segments up front: they decide whether ARP is
    // authoritative for a given target, and which hosts are worth re-priming.
    let locals = netinfo::detect();
    let own_ips: HashSet<Ipv4Addr> = locals.iter().filter_map(|n| n.ip.parse().ok()).collect();
    let local_ranges: Vec<(u32, u32)> = locals
        .iter()
        .filter_map(|n| {
            let ip: Ipv4Addr = n.ip.parse().ok()?;
            let mask = if n.prefix == 0 { 0 } else { u32::MAX << (32 - n.prefix) };
            Some((u32::from(ip) & mask, mask))
        })
        .collect();

    // Read the ARP cache *after* probing: every probe (ping or TCP SYN) forces
    // the OS to ARP-resolve its target, so the cache is now primed. On the local
    // segment ARP is authoritative — a device with a resolved MAC is definitely
    // up, even if it silently dropped our ICMP/TCP probes (phones, IoT, printers,
    // firewalled hosts). This is how a LAN scanner actually finds everything.
    let mut arp = read_arp_cache().await;

    // Second discovery pass — the key to *stable* results. A device that answered
    // a beat slowly, or dropped our single first packet (common on Wi-Fi), has no
    // ARP entry at the instant we read the cache, so it appears one scan and
    // vanishes the next. Re-trigger ARP for every local-segment address that
    // still lacks an entry, let it settle, then read and merge the cache again.
    // This is the software-privilege equivalent of the retried ARP sweep that
    // makes Advanced IP Scanner / Angry IP consistent run to run.
    let range_is_local =
        !arp.is_empty() || probe_results.iter().any(|(ip, _)| ip_in_ranges(*ip, &local_ranges));
    if range_is_local && !cancelled() {
        let needs_prime: Vec<Ipv4Addr> = probe_results
            .iter()
            .map(|(ip, _)| *ip)
            .filter(|ip| !own_ips.contains(ip) && !arp.contains_key(ip))
            .collect();
        if !needs_prime.is_empty() {
            let prime_sem = Arc::new(Semaphore::new(concurrency));
            stream::iter(needs_prime)
                .map(|ip| {
                    let prime_sem = prime_sem.clone();
                    let ports = ports.clone();
                    async move {
                        let _permit = prime_sem.acquire().await.unwrap();
                        arp_prime(ip, &ports, per_probe).await;
                    }
                })
                .buffer_unordered(concurrency)
                .collect::<Vec<()>>()
                .await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            // Union the two reads (latest wins) so entries that resolved in either
            // pass are kept, even if one expired between reads.
            arp.extend(read_arp_cache().await);
        }
    }

    // Guard against proxy-ARP: some routers/APs (and client-isolated Wi-Fi)
    // answer ARP for *every* address in the subnet with their own MAC, which
    // would make the entire scanned range look "up". Any MAC that covers a large
    // share of the range is such a proxy, not a real device — ignore it for
    // ARP-based liveness and for MAC/vendor labelling.
    let mut mac_freq: HashMap<&str, usize> = HashMap::new();
    for (ip, _) in &probe_results {
        if let Some(mac) = arp.get(ip) {
            *mac_freq.entry(mac.as_str()).or_default() += 1;
        }
    }
    let proxy_threshold = (scanned / 16).max(8);
    let is_proxy = |mac: &str| mac_freq.get(mac).copied().unwrap_or(0) > proxy_threshold;
    let has_real_mac = |ip: &Ipv4Addr| arp.get(ip).is_some_and(|m| !is_proxy(m));

    // On the local segment, ARP is the ground truth: a real device answers ARP
    // with its own MAC, which no firewall or middlebox can forge. A transparent
    // router/proxy CAN, however, accept TCP or answer ICMP for *every* address
    // in the subnet (e.g. intercepting DNS on port 53), which would make dead
    // IPs look "up". So for hosts on one of this machine's own subnets we require
    // a real, non-proxy MAC; remote subnets (which have no ARP entries for their
    // hosts) keep the probe-based signals. This is how Angry IP / Advanced IP
    // Scanner avoid counting phantom hosts on a LAN.
    //
    // Even if interface detection missed this subnet, a real resolved MAC for any
    // scanned IP proves the range is on a local segment.
    let arp_in_range = probe_results.iter().any(|(ip, _)| has_real_mac(ip));

    probe_results.retain(|(ip, p)| {
        if own_ips.contains(ip) {
            return true; // this machine itself
        }
        if arp_in_range || ip_in_ranges(*ip, &local_ranges) {
            has_real_mac(ip)
        } else {
            // Remote/routed target: trust ICMP reply / TCP accept / RST.
            p.up
        }
    });

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
            // Don't label a host with a proxy MAC — it's the router's, not the
            // device's, and would be misleading.
            let mac = arp.get(&ip).filter(|m| !is_proxy(m)).cloned();
            let vendor = mac.as_deref().and_then(oui::lookup);
            let os_guess = probe.ttl.and_then(os_from_ttl);
            HostResult {
                ip: ip.to_string(),
                hostname: hostnames.get(&ip).cloned(),
                mac,
                vendor,
                open_ports: probe.open_ports,
                response_ms: probe.response_ms,
                ttl: probe.ttl,
                os_guess,
                last_seen: now.clone(),
            }
        })
        .collect();

    emit(ScanProgress {
        done: scanned,
        total: scanned,
        phase: "done".into(),
    });

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
    ttl: Option<u8>,
}

/// Re-trigger ARP resolution for one address without caring about the result.
/// Any outgoing packet forces the OS to ARP-resolve the destination first, so a
/// second round of connects (plus a ping, for hosts that filter every TCP port)
/// repopulates the neighbour cache for devices that were slow or lossy on the
/// first pass. This is a discovery aid only — no result is read back here.
async fn arp_prime(ip: Ipv4Addr, ports: &[u16], per_probe: Duration) {
    let subset: Vec<u16> = ports.iter().copied().take(4).collect();
    let tcp = async {
        stream::iter(subset)
            .map(|port| async move {
                let _ = tcp_probe(ip, port, per_probe).await;
            })
            .buffer_unordered(4)
            .collect::<Vec<()>>()
            .await;
    };
    let ping = async {
        let _ = icmp_ping(ip, per_probe).await;
    };
    futures::join!(tcp, ping);
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

    let (ping_reply, tcp_results) = futures::join!(ping_fut, tcp_fut);

    let mut open_ports = Vec::new();
    let mut best: Option<u64> = None;
    let mut alive_via_tcp = false;

    if let Some(reply) = &ping_reply {
        best = Some(reply.rtt.as_millis() as u64);
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
        up: ping_reply.is_some() || alive_via_tcp,
        open_ports,
        response_ms: best,
        ttl: ping_reply.and_then(|r| r.ttl),
    }
}

struct PingReply {
    rtt: Duration,
    ttl: Option<u8>,
}

enum PortState {
    Open(Duration),
    Refused(Duration),
    NoReply,
}

/// Map an observed TTL to a coarse OS family. On a LAN the reply TTL is the
/// sender's initial TTL minus a hop or two: ~64 = Linux/Unix/macOS, ~128 =
/// Windows, ~255 = network gear (routers, printers, switches).
fn os_from_ttl(ttl: u8) -> Option<String> {
    let label = if (33..=64).contains(&ttl) {
        "Linux/Unix/macOS"
    } else if (65..=128).contains(&ttl) {
        "Windows"
    } else if ttl > 128 {
        "Network device"
    } else {
        return None;
    };
    Some(label.to_string())
}

/// Parse the TTL value out of a `ping` reply line (case-insensitive `ttl=NN`).
fn parse_ttl(output: &str) -> Option<u8> {
    let lower = output.to_ascii_lowercase();
    let idx = lower.find("ttl=")?;
    let rest = &lower[idx + 4..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
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
/// app never needs administrator/root privileges. Captures the reply so the TTL
/// can be parsed; requiring a `ttl=` marker also filters out the Windows quirk
/// where `ping` exits 0 on a "Destination host unreachable" response.
async fn icmp_ping(ip: Ipv4Addr, per_probe: Duration) -> Option<PingReply> {
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

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    cmd.stdin(std::process::Stdio::null());

    // Guard against a hung ping with a slightly larger outer timeout.
    let outer = per_probe + Duration::from_millis(500);
    match timeout(outer, cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // A genuine echo reply always reports a TTL; require it.
            if !text.to_ascii_lowercase().contains("ttl=") {
                return None;
            }
            Some(PingReply {
                rtt: start.elapsed(),
                ttl: parse_ttl(&text),
            })
        }
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
    fn ip_range_containment() {
        // 192.168.0.0/24
        let ranges = [(u32::from(Ipv4Addr::new(192, 168, 0, 0)), u32::MAX << 8)];
        assert!(ip_in_ranges(Ipv4Addr::new(192, 168, 0, 5), &ranges));
        assert!(ip_in_ranges(Ipv4Addr::new(192, 168, 0, 254), &ranges));
        assert!(!ip_in_ranges(Ipv4Addr::new(192, 168, 1, 5), &ranges));
        assert!(!ip_in_ranges(Ipv4Addr::new(10, 0, 0, 1), &ranges));
    }

    #[test]
    fn parses_ttl_from_ping_output() {
        assert_eq!(parse_ttl("Reply from 1.2.3.4: bytes=32 time=1ms TTL=128"), Some(128));
        assert_eq!(parse_ttl("64 bytes from 10.0.0.1: icmp_seq=0 ttl=64 time=0.4 ms"), Some(64));
        assert_eq!(parse_ttl("no ttl here"), None);
    }

    #[test]
    fn os_guess_from_ttl() {
        assert_eq!(os_from_ttl(64).as_deref(), Some("Linux/Unix/macOS"));
        assert_eq!(os_from_ttl(128).as_deref(), Some("Windows"));
        assert_eq!(os_from_ttl(255).as_deref(), Some("Network device"));
        assert_eq!(os_from_ttl(10), None);
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
