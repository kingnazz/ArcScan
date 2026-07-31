//! The network scanner: liveness detection, port probing, MAC/vendor and
//! hostname resolution. Everything here is read-only discovery — no exploit,
//! brute-force, or credential logic exists or belongs in this module.
//!
//! # Concurrency model
//!
//! Three independent limits bound a scan, because they exhaust three different
//! resources:
//!
//! * **Host concurrency** — how many addresses are worked on at once. Bounds
//!   in-flight bookkeeping and how aggressively the range is swept.
//! * **Global TCP probe concurrency** — how many TCP connection attempts exist
//!   across the *entire* scan. This is the limit that protects file descriptors
//!   and consumer routers, which drop ARP replies (making hosts vanish) when hit
//!   with too much simultaneous fan-out.
//! * **Ping process concurrency** — how many OS `ping` child processes run at
//!   once. Processes are far more expensive than sockets, so this is the
//!   tightest limit.
//!
//! Before this split, host concurrency alone was bounded while each host fanned
//! out to *every* selected port simultaneously, so 64 hosts times 2,048 ports
//! meant over 130,000 concurrent connects.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Notify, Semaphore};
use tokio::time::timeout;

use crate::ipparse;
use crate::netinfo;
use crate::oui;
use crate::ports;

/// True if `ip` falls within any (network, mask) range.
fn ip_in_ranges(ip: Ipv4Addr, ranges: &[(u32, u32)]) -> bool {
    let v = u32::from(ip);
    ranges.iter().any(|(net, mask)| v & mask == *net)
}

/// The scan currently running, and the scan a cancel was requested for.
///
/// Keying cancellation to a scan id means a Stop click that lands after a scan
/// already finished cannot cancel the *next* one. Only one scan runs at a time
/// (the UI disables Scan while one is active), so two counters are enough.
static ACTIVE_SCAN: AtomicU64 = AtomicU64::new(0);
static CANCEL_SCAN: AtomicU64 = AtomicU64::new(0);
static NEXT_SCAN_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate the id used to tag every event of one scan, so the UI can discard
/// events that belong to a scan it is no longer showing.
pub fn next_scan_id() -> u64 {
    NEXT_SCAN_ID.fetch_add(1, Ordering::Relaxed)
}

/// Wakes every cancellation-aware wait the moment a cancel is requested, so
/// Stop interrupts settle delays instead of waiting them out.
fn cancel_notify() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

/// Ask the running scan to stop as soon as it can. It finishes early and still
/// returns the hosts discovered so far, so the user keeps partial results.
pub fn request_cancel() {
    CANCEL_SCAN.store(ACTIVE_SCAN.load(Ordering::Relaxed), Ordering::Relaxed);
    cancel_notify().notify_waiters();
}

fn cancelled(scan_id: u64) -> bool {
    scan_id != 0 && CANCEL_SCAN.load(Ordering::Relaxed) == scan_id
}

/// Sleep that ends early when this scan is cancelled. Returns true if the scan
/// is cancelled, whether that happened before, during or right after the wait.
/// Waits on a notification rather than polling, so Stop lands immediately.
async fn cancellable_sleep(scan_id: u64, dur: Duration) -> bool {
    if cancelled(scan_id) {
        return true;
    }
    let sleep = tokio::time::sleep(dur);
    tokio::pin!(sleep);
    loop {
        // Register interest in the notification before re-checking the flag, so
        // a cancel arriving between the check and the wait cannot be missed.
        let notified = cancel_notify().notified();
        tokio::pin!(notified);
        if cancelled(scan_id) {
            return true;
        }
        tokio::select! {
            _ = &mut sleep => return cancelled(scan_id),
            _ = &mut notified => {
                if cancelled(scan_id) {
                    return true;
                }
            }
        }
    }
}

/// Resolves once this scan is cancelled. Used to keep a blocked event send from
/// pinning a cancelled scan forever.
async fn cancel_requested(scan_id: u64) {
    loop {
        let notified = cancel_notify().notified();
        tokio::pin!(notified);
        if cancelled(scan_id) {
            return;
        }
        notified.await;
    }
}

/// The phase boundaries where a scan re-evaluates cancellation. Tests register a
/// hook on these to trigger a cancel at an exact, deterministic point instead of
/// racing a timer against real network waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint {
    BeforeProbing,
    AfterProbing,
    BeforeArpSettle,
    BeforeConfirm,
    BeforeSecondSettle,
    BeforeDns,
    BeforeFinish,
}

#[cfg(test)]
#[allow(clippy::type_complexity)]
static CHECKPOINT_HOOK: std::sync::Mutex<Option<Box<dyn Fn(Checkpoint) + Send>>> =
    std::sync::Mutex::new(None);

fn checkpoint(cp: Checkpoint) {
    let _ = cp;
    #[cfg(test)]
    {
        if let Some(hook) = CHECKPOINT_HOOK.lock().unwrap().as_ref() {
            hook(cp);
        }
    }
}

/// How many simultaneous operations of each kind a scan may run.
///
/// Defaults are deliberately conservative. Consumer routers and access points
/// rate-limit and drop ARP replies when hit with too much simultaneous fan-out,
/// which makes real hosts vanish from a scan; a gentler sweep resolves more of
/// them in one pass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScanLimits {
    /// Addresses worked on at once.
    pub host_concurrency: usize,
    /// TCP connection attempts in flight across the whole scan.
    pub tcp_concurrency: usize,
    /// OS `ping` child processes running at once.
    pub ping_concurrency: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            host_concurrency: 64,
            tcp_concurrency: 256,
            ping_concurrency: 32,
        }
    }
}

impl ScanLimits {
    /// Clamp operator-supplied limits into ranges the app can actually sustain.
    fn clamped(self) -> Self {
        Self {
            host_concurrency: self.host_concurrency.clamp(1, 1_024),
            tcp_concurrency: self.tcp_concurrency.clamp(8, 2_048),
            ping_concurrency: self.ping_concurrency.clamp(1, 128),
        }
    }

    /// How many ports one host may probe simultaneously.
    ///
    /// The global semaphore is what actually bounds sockets; this only keeps the
    /// number of *pending futures* proportional to the work available, so a
    /// 2,048-port single-host scan is not throttled to a trickle while a /24
    /// sweep does not build a 130,000-future queue.
    fn per_host_fanout(&self, host_count: usize) -> usize {
        let busy_hosts = host_count.min(self.host_concurrency).max(1);
        (self.tcp_concurrency / busy_hosts).clamp(8, 256)
    }
}

/// Probe budget for one scan: addresses times ports.
///
/// Rejecting by workload rather than by address count alone is what stops the
/// combination that actually hurts. A /16 with the 14 default ports is a long
/// but legitimate sweep; a /16 with 2,048 ports is 134 million connection
/// attempts and is always a mistake.
pub const MAX_WORKLOAD: u64 = 4_000_000;

/// Workload above which the scan still runs but the UI shows a warning.
pub const WARN_WORKLOAD: u64 = 500_000;

/// Progress update streamed to the UI while a scan runs.
#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub scan_id: u64,
    pub done: usize,
    pub total: usize,
    /// Live hosts confirmed so far.
    pub found: usize,
    pub phase: ScanPhase,
    pub elapsed_ms: u64,
}

/// The stage a scan is in. Drives the phase label in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanPhase {
    /// Sweeping addresses with ICMP and TCP probes.
    Probing,
    /// Re-triggering ARP for local addresses that did not answer.
    Confirming,
    /// Reading the ARP cache and resolving hostnames and vendors.
    Resolving,
    Done,
    Cancelled,
}

/// Emitted once at the start of a scan so the UI can size its progress display
/// and surface any workload warning before results arrive.
#[derive(Debug, Clone, Serialize)]
pub struct ScanStarted {
    pub scan_id: u64,
    pub target: String,
    pub profile: Option<String>,
    pub total: usize,
    pub port_count: usize,
    /// Non-blocking advisory, e.g. a large but legal workload.
    pub warning: Option<String>,
}

/// How many events the scanner-to-UI channel buffers before applying
/// backpressure. Large enough to absorb a burst of discoveries, small enough
/// that a stalled consumer bounds memory instead of growing a queue forever.
pub const EVENT_CHANNEL_CAPACITY: usize = 512;

/// The scanner's side of the bounded event channel.
///
/// Events fall into two classes. *Critical* events (started, host discovered,
/// host removed, final host update, final phase) must arrive for the streamed
/// table to end up identical to the saved result, so the sink waits for channel
/// capacity — that wait is the backpressure that bounds memory. *Advisory*
/// events (intermediate progress) may be dropped when the channel is full,
/// because a newer progress event supersedes a lost one.
///
/// Two situations must never wedge the scan: the receiver disappearing (the
/// window closed), which makes sends fail immediately and is ignored, and a
/// receiver that stops consuming after the scan was cancelled, which is handled
/// by abandoning the send once cancellation is requested — the returned
/// `ScanResult` remains the source of truth.
#[derive(Clone)]
struct EventSink {
    tx: Option<Sender<ScanEvent>>,
    scan_id: u64,
}

impl EventSink {
    /// Send an event the UI must see, waiting for capacity if the channel is
    /// full. Gives up only when the receiver is gone or the scan is cancelled.
    async fn critical(&self, event: ScanEvent) {
        let Some(tx) = &self.tx else { return };
        match tx.try_send(event) {
            Ok(()) | Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(event)) => {
                tokio::select! {
                    result = tx.send(event) => { let _ = result; }
                    _ = cancel_requested(self.scan_id) => {}
                }
            }
        }
    }

    /// Send a droppable event. Never waits: when the channel is full the event
    /// is discarded, because a later one carries fresher information.
    fn advisory(&self, event: ScanEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(event);
        }
    }
}

/// Everything the scanner streams to the command layer while running.
#[derive(Debug, Clone, Serialize)]
pub enum ScanEvent {
    Started(ScanStarted),
    Progress(ScanProgress),
    /// A live host was found. Fields other than the address may still be empty.
    HostDiscovered {
        scan_id: u64,
        host: Box<HostResult>,
    },
    /// An already-reported host gained MAC, vendor, hostname or OS information.
    HostUpdated {
        scan_id: u64,
        host: Box<HostResult>,
    },
    /// A host reported during probing turned out not to be real (see the
    /// proxy-ARP and local-segment rules in [`run`]). Sent so the streamed table
    /// always ends up identical to the saved result.
    HostRemoved {
        scan_id: u64,
        ip: String,
    },
}

/// Options for one scan. Field names are stable: `concurrency` keeps its v1.6
/// meaning of *host* concurrency so older saved preferences still apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub target: String,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_host_concurrency")]
    pub concurrency: usize,
    /// Global TCP probe ceiling. Falls back to the default when absent.
    #[serde(default)]
    pub tcp_concurrency: Option<usize>,
    /// Ping process ceiling. Falls back to the default when absent.
    #[serde(default)]
    pub ping_concurrency: Option<usize>,
    /// Name of the profile the options came from, recorded with the scan so
    /// comparisons only ever line up like with like.
    #[serde(default)]
    pub profile: Option<String>,
    /// `Some(false)` forces routed-scan behaviour: no ARP-based liveness and no
    /// re-prime pass, for targets the operator knows are remote. `None` decides
    /// automatically from the detected local subnets and the ARP cache.
    #[serde(default)]
    pub arp_assist: Option<bool>,
}

fn default_timeout() -> u64 {
    900
}

fn default_host_concurrency() -> usize {
    ScanLimits::default().host_concurrency
}

impl ScanOptions {
    fn limits(&self) -> ScanLimits {
        let d = ScanLimits::default();
        ScanLimits {
            host_concurrency: self.concurrency,
            tcp_concurrency: self.tcp_concurrency.unwrap_or(d.tcp_concurrency),
            ping_concurrency: self.ping_concurrency.unwrap_or(d.ping_concurrency),
        }
        .clamped()
    }
}

/// One host as observed by a single scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostResult {
    pub ip: String,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<u16>,
    /// Fastest response of any kind, in whole milliseconds. Kept for exports and
    /// databases written by earlier versions; prefer the two measurements below.
    pub response_ms: Option<u64>,
    /// ICMP round-trip time as reported by the OS `ping` output.
    #[serde(default)]
    pub icmp_ms: Option<f64>,
    /// Fastest TCP connection establishment time across the probed ports.
    #[serde(default)]
    pub tcp_ms: Option<f64>,
    /// TTL from the ICMP echo reply, when a ping succeeded.
    pub ttl: Option<u8>,
    /// Coarse OS guess derived from the TTL.
    pub os_guess: Option<String>,
    pub last_seen: String,
}

impl HostResult {
    fn new(ip: Ipv4Addr, probe: &Probe, seen_at: &str) -> Self {
        let mut host = HostResult {
            ip: ip.to_string(),
            hostname: None,
            mac: None,
            vendor: None,
            open_ports: probe.open_ports.clone(),
            response_ms: None,
            icmp_ms: probe.icmp_ms,
            tcp_ms: probe.tcp_ms,
            ttl: probe.ttl,
            os_guess: probe.ttl.and_then(os_from_ttl),
            last_seen: seen_at.to_string(),
        };
        host.response_ms = host.fastest_ms();
        host
    }

    /// Fastest observed response in whole milliseconds, rounded up so a
    /// sub-millisecond reply reads as `1 ms` rather than `0 ms`.
    fn fastest_ms(&self) -> Option<u64> {
        let best = match (self.icmp_ms, self.tcp_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }?;
        Some(best.ceil().max(1.0) as u64)
    }
}

/// Result of a completed or cancelled scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub scan_id: u64,
    pub target: String,
    #[serde(default)]
    pub profile: Option<String>,
    pub duration_ms: u64,
    /// Addresses enumerated by the target.
    pub scanned: usize,
    /// Addresses actually probed. Lower than `scanned` for a cancelled scan.
    #[serde(default)]
    pub probed: usize,
    pub hosts: Vec<HostResult>,
    /// True when the operator stopped the scan before it finished.
    #[serde(default)]
    pub cancelled: bool,
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

/// A validated scan, ready to run. Produced by [`plan`] so the command layer can
/// reject bad input and report the workload before any probe is sent.
#[derive(Debug, Clone)]
pub struct ScanPlan {
    pub hosts: Vec<Ipv4Addr>,
    pub ports: Vec<u16>,
    pub limits: ScanLimits,
    pub timeout: Duration,
    pub workload: u64,
    pub warning: Option<String>,
}

/// Validate a scan request. Every limit that matters is enforced here, in Rust,
/// so the backend never depends on the frontend having done the same checks.
pub fn plan(opts: &ScanOptions) -> Result<ScanPlan, String> {
    let hosts = ipparse::parse_target(&opts.target)?;
    let ports = ports::sanitize(&opts.ports)?;
    let limits = opts.limits();
    let timeout_ms = opts.timeout_ms.clamp(50, 10_000);

    let workload = hosts.len() as u64 * ports.len() as u64;
    if workload > MAX_WORKLOAD {
        return Err(format!(
            "This scan would make {} connection attempts ({} addresses x {} ports), \
             past the {} attempt limit. Narrow the address range or the port list.",
            thousands(workload),
            thousands(hosts.len() as u64),
            ports.len(),
            thousands(MAX_WORKLOAD),
        ));
    }
    let warning = (workload > WARN_WORKLOAD).then(|| {
        format!(
            "Large scan: {} connection attempts across {} addresses. This can take a while \
             and puts sustained load on your network.",
            thousands(workload),
            thousands(hosts.len() as u64),
        )
    });

    Ok(ScanPlan {
        hosts,
        ports,
        limits,
        timeout: Duration::from_millis(timeout_ms),
        workload,
        warning,
    })
}

/// Group digits so large numbers stay readable in an error message.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Run a full scan, streaming events to `events` as hosts are discovered and
/// enriched. `scan_id` tags every event and scopes cancellation.
///
/// Cancellation is re-evaluated before every phase, during every wait, and once
/// more immediately before the result is built, so the returned `cancelled`
/// flag reflects the scan's true final state rather than a value captured after
/// probing. A scan stopped mid-way keeps every host discovered so far, along
/// with whatever MAC, vendor and hostname data had already resolved.
pub async fn run(
    opts: ScanOptions,
    scan_id: u64,
    events: Option<Sender<ScanEvent>>,
) -> Result<ScanResult, String> {
    let ScanPlan {
        hosts,
        ports,
        limits,
        timeout: per_probe,
        warning,
        ..
    } = plan(&opts)?;

    // Take ownership of the cancellation slot for this scan, clearing any stale
    // request left by a previous one.
    ACTIVE_SCAN.store(scan_id, Ordering::Relaxed);
    CANCEL_SCAN.store(0, Ordering::Relaxed);

    let started = Instant::now();
    let scanned = hosts.len();
    let sink = EventSink {
        tx: events,
        scan_id,
    };

    sink.critical(ScanEvent::Started(ScanStarted {
        scan_id,
        target: opts.target.clone(),
        profile: opts.profile.clone(),
        total: scanned,
        port_count: ports.len(),
        warning,
    }))
    .await;
    checkpoint(Checkpoint::BeforeProbing);

    // Detect our own local segments up front: they decide whether ARP is
    // authoritative for this target, and therefore whether the re-prime pass is
    // worth running at all.
    let locals = netinfo::detect();
    let own_ips: HashSet<Ipv4Addr> = locals.iter().filter_map(|n| n.ip.parse().ok()).collect();
    let local_ranges: Vec<(u32, u32)> = locals
        .iter()
        .filter_map(|n| {
            let ip: Ipv4Addr = n.ip.parse().ok()?;
            let mask = if n.prefix == 0 {
                0
            } else {
                u32::MAX << (32 - n.prefix)
            };
            Some((u32::from(ip) & mask, mask))
        })
        .collect();
    let overlaps_local_subnet = hosts.iter().any(|ip| ip_in_ranges(*ip, &local_ranges));

    let tcp_sem = Arc::new(Semaphore::new(limits.tcp_concurrency));
    let ping_sem = Arc::new(Semaphore::new(limits.ping_concurrency));
    let ports = Arc::new(ports);
    let fanout = limits.per_host_fanout(scanned);

    // Probe every address. Progress and discoveries stream out as they land.
    let counters = Arc::new(Counters::default());
    let probe_started = Instant::now();
    let mut probe_results: Vec<(Ipv4Addr, Probe)> = stream::iter(hosts)
        .map(|ip| {
            let ports = Arc::clone(&ports);
            let tcp_sem = Arc::clone(&tcp_sem);
            let ping_sem = Arc::clone(&ping_sem);
            let counters = Arc::clone(&counters);
            let sink = sink.clone();
            async move {
                // Once cancelled, drain the remaining addresses without probing
                // so the stream finishes immediately instead of running to the
                // end of the range.
                let probe = if cancelled(scan_id) {
                    Probe::dead()
                } else {
                    let probe = probe_host(
                        ip,
                        &ports,
                        per_probe,
                        fanout,
                        Arc::clone(&tcp_sem),
                        Arc::clone(&ping_sem),
                    )
                    .await;
                    counters.probed.fetch_add(1, Ordering::Relaxed);
                    if probe.up {
                        counters.found.fetch_add(1, Ordering::Relaxed);
                        let now = chrono::Local::now().to_rfc3339();
                        sink.critical(ScanEvent::HostDiscovered {
                            scan_id,
                            host: Box::new(HostResult::new(ip, &probe, &now)),
                        })
                        .await;
                    }
                    probe
                };
                let done = counters.done.fetch_add(1, Ordering::Relaxed) + 1;
                if counters.should_report(done, scanned) {
                    sink.advisory(ScanEvent::Progress(ScanProgress {
                        scan_id,
                        done,
                        total: scanned,
                        found: counters.found.load(Ordering::Relaxed),
                        phase: ScanPhase::Probing,
                        elapsed_ms: probe_started.elapsed().as_millis() as u64,
                    }));
                }
                (ip, probe)
            }
        })
        .buffer_unordered(limits.host_concurrency)
        .collect()
        .await;

    checkpoint(Checkpoint::AfterProbing);
    let probed = counters.probed.load(Ordering::Relaxed);
    let progress_at = |phase: ScanPhase| ScanProgress {
        scan_id,
        done: counters.done.load(Ordering::Relaxed),
        total: scanned,
        found: counters.found.load(Ordering::Relaxed),
        phase,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };

    // Read the ARP cache *after* probing: every probe (ping or TCP SYN) forces
    // the OS to ARP-resolve its target, so the cache is now primed. On the local
    // segment ARP is authoritative — a device with a resolved MAC is definitely
    // up, even if it silently dropped our ICMP/TCP probes (phones, IoT,
    // printers, firewalled hosts). This is how a LAN scanner finds everything.
    //
    // A cancelled scan skips the settle delay so Stop feels immediate; the cache
    // is still read so whatever was found keeps its MAC and vendor.
    sink.advisory(ScanEvent::Progress(progress_at(ScanPhase::Resolving)));
    checkpoint(Checkpoint::BeforeArpSettle);
    // Let late ARP replies from slow devices settle before reading, so one scan
    // captures them instead of trickling across repeated scans. The wait ends
    // the moment Stop is pressed.
    cancellable_sleep(scan_id, Duration::from_millis(700)).await;
    let mut arp = read_arp_cache().await;

    // Is ARP authoritative for *this* target? Two pieces of evidence count:
    // the range overlaps one of our own subnets, or one of the addresses we
    // actually scanned resolved to a real, non-proxy MAC.
    //
    // Unrelated entries in the ARP cache are NOT evidence. Every machine has a
    // gateway entry, so treating a non-empty cache as proof of locality (which
    // v1.6 did) made every remote scan run a pointless re-prime pass and applied
    // local-segment liveness rules to routed targets.
    let target_arp_evidence = |arp: &HashMap<Ipv4Addr, String>| {
        let freq = proxy_frequencies(arp, probe_results.iter().map(|(ip, _)| *ip));
        let threshold = proxy_threshold(scanned);
        probe_results
            .iter()
            .any(|(ip, _)| is_real_mac(arp, &freq, threshold, ip))
    };
    let arp_authoritative = match opts.arp_assist {
        // The Remote subnet profile opts out of every local ARP assumption.
        Some(false) => false,
        Some(true) => true,
        None => overlaps_local_subnet || target_arp_evidence(&arp),
    };

    // Second discovery pass — the key to *stable* results. A device that
    // answered a beat slowly, or dropped our first packet (common on Wi-Fi), has
    // no ARP entry at the instant we read the cache, so it appears in one scan
    // and vanishes from the next. Re-trigger ARP for every local address that
    // still lacks an entry, let it settle, then read and merge the cache again.
    //
    // Routed targets never get here: they have no ARP entries to repair, so the
    // pass would be pure wasted traffic. A cancelled scan skips the pass
    // entirely — confirmation is expensive follow-up work, not preservation.
    checkpoint(Checkpoint::BeforeConfirm);
    if arp_authoritative && !cancelled(scan_id) {
        let needs_prime: Vec<Ipv4Addr> = probe_results
            .iter()
            .map(|(ip, _)| *ip)
            .filter(|ip| !own_ips.contains(ip) && !arp.contains_key(ip))
            .collect();
        if !needs_prime.is_empty() {
            sink.advisory(ScanEvent::Progress(progress_at(ScanPhase::Confirming)));
            stream::iter(needs_prime)
                .map(|ip| {
                    let ports = Arc::clone(&ports);
                    let tcp_sem = Arc::clone(&tcp_sem);
                    let ping_sem = Arc::clone(&ping_sem);
                    async move {
                        if !cancelled(scan_id) {
                            arp_prime(ip, &ports, per_probe, tcp_sem, ping_sem).await;
                        }
                    }
                })
                .buffer_unordered(limits.host_concurrency)
                .collect::<Vec<()>>()
                .await;
            checkpoint(Checkpoint::BeforeSecondSettle);
            cancellable_sleep(scan_id, Duration::from_millis(600)).await;
            // Union the two reads (latest wins) so entries that resolved in
            // either pass are kept, even if one expired between reads.
            arp.extend(read_arp_cache().await);
            sink.advisory(ScanEvent::Progress(progress_at(ScanPhase::Resolving)));
        }
    }

    // Guard against proxy-ARP: some routers and APs (and client-isolated Wi-Fi)
    // answer ARP for *every* address in the subnet with their own MAC, which
    // would make the whole scanned range look "up". Any MAC covering a large
    // share of the range is such a proxy, not a real device.
    let mac_freq = proxy_frequencies(&arp, probe_results.iter().map(|(ip, _)| *ip));
    let threshold = proxy_threshold(scanned);
    let has_real_mac = |ip: &Ipv4Addr| is_real_mac(&arp, &mac_freq, threshold, ip);

    // On the local segment ARP is ground truth: a real device answers ARP with
    // its own MAC, which no firewall or middlebox can forge. A transparent
    // router CAN, however, accept TCP or answer ICMP for *every* address in the
    // subnet (intercepting port 53, for instance), which would make dead
    // addresses look up. So for local targets we require a real, non-proxy MAC;
    // routed targets, which have no ARP entries at all, keep the probe signals.
    let mut removed: Vec<String> = Vec::new();
    probe_results.retain(|(ip, p)| {
        let keep = if own_ips.contains(ip) {
            true // this machine itself
        } else if arp_authoritative {
            has_real_mac(ip)
        } else {
            p.up
        };
        // A host streamed as discovered that does not survive the local-segment
        // rule must be withdrawn, or the live table would disagree with what
        // gets saved.
        if !keep && p.up {
            removed.push(ip.to_string());
        }
        keep
    });
    for ip in removed {
        sink.critical(ScanEvent::HostRemoved { scan_id, ip }).await;
    }

    probe_results.sort_by_key(|(ip, _)| u32::from(*ip));

    // Resolve hostnames for the live hosts concurrently (bounded, with a short
    // per-lookup timeout) so N slow reverse-DNS misses collapse into one pass.
    // A cancelled scan launches no lookups at all: each queued lookup re-checks
    // cancellation first, so lookups already in flight finish within their own
    // short timeout while the rest are skipped. Hostnames that resolved before
    // the cancel are kept.
    checkpoint(Checkpoint::BeforeDns);
    let dns_concurrency = limits.tcp_concurrency.min(128);
    let dns_sem = Arc::new(Semaphore::new(dns_concurrency));
    let hostnames: HashMap<Ipv4Addr, String> =
        stream::iter(probe_results.iter().map(|(ip, _)| *ip).collect::<Vec<_>>())
            .map(|ip| {
                let dns_sem = Arc::clone(&dns_sem);
                async move {
                    if cancelled(scan_id) {
                        return None;
                    }
                    let _permit = dns_sem.acquire().await.ok()?;
                    if cancelled(scan_id) {
                        return None;
                    }
                    resolve_hostname(ip).await.map(|name| (ip, name))
                }
            })
            .buffer_unordered(dns_concurrency)
            .filter_map(|pair| async move { pair })
            .collect()
            .await;

    // Final enrichment works from data already in memory (the ARP cache read and
    // the resolved hostnames), so it runs even for a cancelled scan: partial
    // results keep every MAC, vendor and hostname that had already resolved.
    let now = chrono::Local::now().to_rfc3339();
    let mut hosts_out: Vec<HostResult> = Vec::with_capacity(probe_results.len());
    for (ip, probe) in probe_results {
        let mut host = HostResult::new(ip, &probe, &now);
        // Never label a host with a proxy MAC: it is the router's, not the
        // device's, and would be misleading.
        host.mac = arp.get(&ip).filter(|_| has_real_mac(&ip)).cloned();
        host.vendor = host.mac.as_deref().and_then(oui::lookup);
        host.hostname = hostnames.get(&ip).cloned();
        sink.critical(ScanEvent::HostUpdated {
            scan_id,
            host: Box::new(host.clone()),
        })
        .await;
        hosts_out.push(host);
    }

    // The final cancellation state is decided here, at the very end, so a Stop
    // pressed during any later phase is honoured rather than a stale value
    // captured after probing.
    checkpoint(Checkpoint::BeforeFinish);
    let was_cancelled = cancelled(scan_id);
    sink.critical(ScanEvent::Progress(progress_at(if was_cancelled {
        ScanPhase::Cancelled
    } else {
        ScanPhase::Done
    })))
    .await;

    ACTIVE_SCAN.store(0, Ordering::Relaxed);

    Ok(ScanResult {
        scan_id,
        target: opts.target,
        profile: opts.profile,
        duration_ms: started.elapsed().as_millis() as u64,
        scanned,
        probed,
        hosts: hosts_out,
        cancelled: was_cancelled,
    })
}

/// Progress bookkeeping shared by the probe tasks.
#[derive(Default)]
struct Counters {
    done: std::sync::atomic::AtomicUsize,
    probed: std::sync::atomic::AtomicUsize,
    found: std::sync::atomic::AtomicUsize,
}

impl Counters {
    /// Throttle progress events to roughly 100 per scan plus the final one, so a
    /// 65k-address sweep does not push 65k messages through the event bridge.
    fn should_report(&self, done: usize, total: usize) -> bool {
        let step = (total / 100).max(1);
        done == total || done % step == 0
    }
}

/// A MAC covering more than this share of the scanned range is a proxy-ARP
/// responder rather than a device.
fn proxy_threshold(scanned: usize) -> usize {
    (scanned / 16).max(8)
}

fn proxy_frequencies(
    arp: &HashMap<Ipv4Addr, String>,
    scanned: impl Iterator<Item = Ipv4Addr>,
) -> HashMap<String, usize> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for ip in scanned {
        if let Some(mac) = arp.get(&ip) {
            *freq.entry(mac.clone()).or_default() += 1;
        }
    }
    freq
}

/// True when `ip` has an ARP entry that belongs to a real device rather than a
/// proxy-ARP responder.
fn is_real_mac(
    arp: &HashMap<Ipv4Addr, String>,
    freq: &HashMap<String, usize>,
    threshold: usize,
    ip: &Ipv4Addr,
) -> bool {
    arp.get(ip)
        .is_some_and(|mac| freq.get(mac).copied().unwrap_or(0) <= threshold)
}

#[derive(Debug, Clone)]
struct Probe {
    up: bool,
    open_ports: Vec<u16>,
    icmp_ms: Option<f64>,
    tcp_ms: Option<f64>,
    ttl: Option<u8>,
}

impl Probe {
    fn dead() -> Self {
        Probe {
            up: false,
            open_ports: Vec::new(),
            icmp_ms: None,
            tcp_ms: None,
            ttl: None,
        }
    }
}

/// Re-trigger ARP resolution for one address without caring about the result.
/// Any outgoing packet forces the OS to ARP-resolve the destination first, so a
/// second round of connects (plus a ping, for hosts that filter every TCP port)
/// repopulates the neighbour cache for devices that were slow or lossy on the
/// first pass. This is a discovery aid only — no result is read back here.
async fn arp_prime(
    ip: Ipv4Addr,
    ports: &[u16],
    per_probe: Duration,
    tcp_sem: Arc<Semaphore>,
    ping_sem: Arc<Semaphore>,
) {
    let subset: Vec<u16> = ports.iter().copied().take(4).collect();
    let tcp = async {
        stream::iter(subset)
            .map(|port| {
                let tcp_sem = Arc::clone(&tcp_sem);
                async move {
                    let _ = tcp_probe(ip, port, per_probe, tcp_sem).await;
                }
            })
            .buffer_unordered(4)
            .collect::<Vec<()>>()
            .await;
    };
    let ping = async {
        let _ = icmp_ping(ip, per_probe, ping_sem).await;
    };
    futures::join!(tcp, ping);
}

/// Probe one address: one ICMP echo plus a bounded TCP fan-out across the
/// selected ports. Both kinds of probe take a permit from their global
/// semaphore, so the totals stay bounded no matter how many hosts are in flight.
async fn probe_host(
    ip: Ipv4Addr,
    ports: &[u16],
    per_probe: Duration,
    fanout: usize,
    tcp_sem: Arc<Semaphore>,
    ping_sem: Arc<Semaphore>,
) -> Probe {
    let ping_fut = icmp_ping(ip, per_probe, ping_sem);
    let tcp_fut = async {
        stream::iter(ports.iter().copied())
            .map(|port| {
                let tcp_sem = Arc::clone(&tcp_sem);
                async move { (port, tcp_probe(ip, port, per_probe, tcp_sem).await) }
            })
            .buffer_unordered(fanout.min(ports.len().max(1)))
            .collect::<Vec<_>>()
            .await
    };

    let (ping_reply, tcp_results) = futures::join!(ping_fut, tcp_fut);

    let mut open_ports = Vec::new();
    let mut tcp_ms: Option<f64> = None;
    let mut alive_via_tcp = false;
    let note = |ms: f64, best: &mut Option<f64>| {
        *best = Some(best.map_or(ms, |b: f64| b.min(ms)));
    };

    for (port, state) in tcp_results {
        match state {
            PortState::Open(d) => {
                open_ports.push(port);
                alive_via_tcp = true;
                note(millis(d), &mut tcp_ms);
            }
            // A refused connection (RST) still proves the host is alive.
            PortState::Refused(d) => {
                alive_via_tcp = true;
                note(millis(d), &mut tcp_ms);
            }
            PortState::NoReply => {}
        }
    }

    open_ports.sort_unstable();
    Probe {
        up: ping_reply.is_some() || alive_via_tcp,
        open_ports,
        icmp_ms: ping_reply.as_ref().map(|r| r.rtt_ms),
        tcp_ms,
        ttl: ping_reply.and_then(|r| r.ttl),
    }
}

/// Duration as fractional milliseconds, rounded to two decimals so sub-
/// millisecond LAN responses stay meaningful without noisy precision.
fn millis(d: Duration) -> f64 {
    (d.as_secs_f64() * 100_000.0).round() / 100.0
}

struct PingReply {
    rtt_ms: f64,
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

/// Parse the round-trip time reported by `ping` itself, which is the actual ICMP
/// latency rather than how long the child process took to start, run and exit.
///
/// Handles the three formats ArcScan ships on:
///
/// * Windows: `Reply from 10.0.0.1: bytes=32 time=3ms TTL=64`, and `time<1ms`
///   for sub-millisecond replies.
/// * macOS and Linux: `64 bytes from 10.0.0.1: icmp_seq=0 ttl=64 time=0.443 ms`
///
/// Returns `None` for localised or unexpected output, so the caller can fall
/// back to the measured process duration.
fn parse_rtt_ms(output: &str) -> Option<f64> {
    let lower = output.to_ascii_lowercase();
    let idx = lower.find("time")?;
    let rest = &lower[idx + 4..];
    // `time=0.443 ms`, `time<1ms`, `time = 3ms`
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').or_else(|| rest.strip_prefix('<'))?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = digits.parse().ok()?;
    value.is_finite().then_some(value)
}

async fn tcp_probe(
    ip: Ipv4Addr,
    port: u16,
    per_probe: Duration,
    tcp_sem: Arc<Semaphore>,
) -> PortState {
    // The global permit is taken *before* the socket is created, so the number
    // of simultaneous connection attempts across the whole scan never exceeds
    // the configured ceiling.
    let Ok(_permit) = tcp_sem.acquire().await else {
        return PortState::NoReply;
    };
    let addr = SocketAddr::new(IpAddr::V4(ip), port);
    let start = Instant::now();
    match timeout(per_probe, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => PortState::Open(start.elapsed()),
        Ok(Err(e)) => match e.kind() {
            // Actively refused == host is up but the port is closed.
            std::io::ErrorKind::ConnectionRefused => PortState::Refused(start.elapsed()),
            _ => PortState::NoReply,
        },
        Err(_) => PortState::NoReply, // timed out
    }
}

/// ICMP echo via the OS `ping` binary — deliberately avoids raw sockets so the
/// app never needs administrator or root privileges. The reply is captured so
/// the TTL and the reported round-trip time can be parsed; requiring a `ttl=`
/// marker also filters out the Windows quirk where `ping` exits 0 on a
/// "Destination host unreachable" response.
async fn icmp_ping(
    ip: Ipv4Addr,
    per_probe: Duration,
    ping_sem: Arc<Semaphore>,
) -> Option<PingReply> {
    // Child processes are the most expensive thing a scan does, so they get the
    // tightest global limit. Without it a wide sweep spawns hundreds at once.
    let _permit = ping_sem.acquire().await.ok()?;

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
                // Prefer the RTT ping itself reports. Process startup and exit
                // add several milliseconds on every platform, so the measured
                // duration is only a fallback for output we cannot parse.
                rtt_ms: parse_rtt_ms(&text).unwrap_or_else(|| millis(start.elapsed())),
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
pub fn normalize_mac(tok: &str) -> Option<String> {
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

    fn opts(target: &str) -> ScanOptions {
        ScanOptions {
            target: target.into(),
            ports: Vec::new(),
            timeout_ms: 900,
            concurrency: 64,
            tcp_concurrency: None,
            ping_concurrency: None,
            profile: None,
            arp_assist: None,
        }
    }

    fn arp_map(pairs: &[(&str, &str)]) -> HashMap<Ipv4Addr, String> {
        pairs
            .iter()
            .map(|(ip, mac)| (ip.parse().unwrap(), (*mac).to_string()))
            .collect()
    }

    /// The locality decision extracted from `run` so it can be tested without
    /// touching the network: a target is local when it overlaps one of our own
    /// subnets, or when an address we actually scanned has a real, non-proxy
    /// ARP entry.
    fn is_local(
        scanned: &[Ipv4Addr],
        local_ranges: &[(u32, u32)],
        arp: &HashMap<Ipv4Addr, String>,
    ) -> bool {
        if scanned.iter().any(|ip| ip_in_ranges(*ip, local_ranges)) {
            return true;
        }
        let freq = proxy_frequencies(arp, scanned.iter().copied());
        let threshold = proxy_threshold(scanned.len());
        scanned
            .iter()
            .any(|ip| is_real_mac(arp, &freq, threshold, ip))
    }

    fn range(base: &str, prefix: u32) -> (u32, u32) {
        let mask = u32::MAX << (32 - prefix);
        (u32::from(base.parse::<Ipv4Addr>().unwrap()) & mask, mask)
    }

    fn hosts(list: &[&str]) -> Vec<Ipv4Addr> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

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
    fn parses_unix_arp() {
        let sample = "router.lan (192.168.0.1) at 3c:37:86:aa:bb:cc [ether] on eth0\n? (192.168.0.44) at 00:1a:2b:3c:4d:5e [ether] on eth0\n";
        let map = parse_arp(sample);
        assert_eq!(
            map.get(&"192.168.0.1".parse().unwrap()).map(String::as_str),
            Some("3C:37:86:AA:BB:CC")
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parses_macos_unpadded_mac() {
        let sample = "gateway.lan (10.0.1.1) at a0:ce:c8:d:cf:d1 on en0 ifscope [ethernet]\n";
        let map = parse_arp(sample);
        assert_eq!(
            map.get(&"10.0.1.1".parse().unwrap()).map(String::as_str),
            Some("A0:CE:C8:0D:CF:D1")
        );
    }

    #[test]
    fn ip_range_containment() {
        let ranges = [range("192.168.0.0", 24)];
        assert!(ip_in_ranges(Ipv4Addr::new(192, 168, 0, 5), &ranges));
        assert!(ip_in_ranges(Ipv4Addr::new(192, 168, 0, 254), &ranges));
        assert!(!ip_in_ranges(Ipv4Addr::new(192, 168, 1, 5), &ranges));
        assert!(!ip_in_ranges(Ipv4Addr::new(10, 0, 0, 1), &ranges));
    }

    #[test]
    fn local_subnet_scan_is_local() {
        let scanned = hosts(&["192.168.1.10", "192.168.1.11"]);
        let locals = [range("192.168.1.0", 24)];
        assert!(is_local(&scanned, &locals, &arp_map(&[])));
    }

    #[test]
    fn remote_scan_is_not_local_despite_unrelated_arp_entries() {
        // The regression this replaces: every machine has a gateway in its ARP
        // cache, so a non-empty cache used to mark even a routed target local,
        // forcing a pointless re-prime pass and local liveness rules.
        let scanned = hosts(&["8.8.8.8", "8.8.4.4"]);
        let locals = [range("192.168.1.0", 24)];
        let arp = arp_map(&[
            ("192.168.1.1", "AA:BB:CC:00:00:01"),
            ("192.168.1.20", "AA:BB:CC:00:00:02"),
        ]);
        assert!(!is_local(&scanned, &locals, &arp));
    }

    #[test]
    fn empty_arp_cache_and_remote_target_is_not_local() {
        let scanned = hosts(&["203.0.113.5"]);
        let locals = [range("10.0.0.0", 24)];
        assert!(!is_local(&scanned, &locals, &arp_map(&[])));
    }

    #[test]
    fn arp_entry_for_a_scanned_target_proves_locality() {
        // Interface detection can miss a subnet (VPNs, bridged adapters). A real
        // MAC for an address we actually scanned is still proof it is local.
        let scanned = hosts(&["172.20.5.9"]);
        let locals: [(u32, u32); 0] = [];
        let arp = arp_map(&[("172.20.5.9", "AA:BB:CC:DD:EE:01")]);
        assert!(is_local(&scanned, &locals, &arp));
    }

    #[test]
    fn proxy_arp_entries_do_not_prove_locality() {
        // A router answering ARP for the whole range with one MAC is not
        // evidence of real devices, so it must not flip a remote scan to local.
        let scanned: Vec<Ipv4Addr> = (1..=64u8).map(|n| Ipv4Addr::new(203, 0, 113, n)).collect();
        let pairs: Vec<(String, String)> = scanned
            .iter()
            .map(|ip| (ip.to_string(), "AA:BB:CC:DD:EE:FF".to_string()))
            .collect();
        let arp: HashMap<Ipv4Addr, String> = pairs
            .iter()
            .map(|(ip, mac)| (ip.parse().unwrap(), mac.clone()))
            .collect();
        let locals: [(u32, u32); 0] = [];
        assert!(!is_local(&scanned, &locals, &arp));
    }

    #[test]
    fn proxy_arp_masks_only_the_shared_mac() {
        // The proxy responder is ignored, but a genuine device on the same
        // segment keeps its MAC.
        let scanned: Vec<Ipv4Addr> = (1..=64u8).map(|n| Ipv4Addr::new(10, 1, 1, n)).collect();
        let mut arp: HashMap<Ipv4Addr, String> = scanned
            .iter()
            .map(|ip| (*ip, "AA:BB:CC:DD:EE:FF".to_string()))
            .collect();
        let real: Ipv4Addr = "10.1.1.7".parse().unwrap();
        arp.insert(real, "11:22:33:44:55:66".into());

        let freq = proxy_frequencies(&arp, scanned.iter().copied());
        let threshold = proxy_threshold(scanned.len());
        assert!(is_real_mac(&arp, &freq, threshold, &real));
        assert!(!is_real_mac(
            &arp,
            &freq,
            threshold,
            &"10.1.1.8".parse().unwrap()
        ));
    }

    #[test]
    fn arp_assist_override_forces_remote_behaviour() {
        let mut o = opts("192.168.1.0/24");
        o.arp_assist = Some(false);
        assert_eq!(o.arp_assist, Some(false));
        // The plan itself is unaffected; only the liveness rule changes.
        assert_eq!(plan(&o).unwrap().hosts.len(), 254);
    }

    #[test]
    fn parses_ttl_from_ping_output() {
        assert_eq!(
            parse_ttl("Reply from 1.2.3.4: bytes=32 time=1ms TTL=128"),
            Some(128)
        );
        assert_eq!(
            parse_ttl("64 bytes from 10.0.0.1: icmp_seq=0 ttl=64 time=0.4 ms"),
            Some(64)
        );
        assert_eq!(parse_ttl("no ttl here"), None);
    }

    #[test]
    fn parses_reported_rtt_across_platforms() {
        // Windows
        assert_eq!(
            parse_rtt_ms("Reply from 10.0.0.1: bytes=32 time=3ms TTL=64"),
            Some(3.0)
        );
        // Windows sub-millisecond
        assert_eq!(
            parse_rtt_ms("Reply from 10.0.0.1: bytes=32 time<1ms TTL=64"),
            Some(1.0)
        );
        // Linux
        assert_eq!(
            parse_rtt_ms("64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=0.443 ms"),
            Some(0.443)
        );
        // macOS
        assert_eq!(
            parse_rtt_ms("64 bytes from 10.0.1.1: icmp_seq=0 ttl=64 time=2.104 ms"),
            Some(2.104)
        );
        // Unparseable / localised output falls back to the measured duration.
        assert_eq!(parse_rtt_ms("Antwort von 10.0.0.1: Bytes=32 TTL=64"), None);
        assert_eq!(parse_rtt_ms(""), None);
    }

    #[test]
    fn os_guess_from_ttl() {
        assert_eq!(os_from_ttl(64).as_deref(), Some("Linux/Unix/macOS"));
        assert_eq!(os_from_ttl(128).as_deref(), Some("Windows"));
        assert_eq!(os_from_ttl(255).as_deref(), Some("Network device"));
        assert_eq!(os_from_ttl(10), None);
    }

    #[test]
    fn response_ms_is_the_fastest_of_both_measurements() {
        let probe = Probe {
            up: true,
            open_ports: vec![443],
            icmp_ms: Some(4.2),
            tcp_ms: Some(1.4),
            ttl: Some(64),
        };
        let host = HostResult::new("10.0.0.5".parse().unwrap(), &probe, "now");
        assert_eq!(host.icmp_ms, Some(4.2));
        assert_eq!(host.tcp_ms, Some(1.4));
        assert_eq!(host.response_ms, Some(2));
    }

    #[test]
    fn sub_millisecond_response_never_reads_as_zero() {
        let probe = Probe {
            up: true,
            open_ports: vec![],
            icmp_ms: Some(0.21),
            tcp_ms: None,
            ttl: None,
        };
        let host = HostResult::new("10.0.0.6".parse().unwrap(), &probe, "now");
        assert_eq!(host.response_ms, Some(1));
    }

    #[test]
    fn plan_dedupes_and_validates_ports() {
        let mut o = opts("10.0.0.1");
        o.ports = vec![443, 80, 443, 80];
        assert_eq!(plan(&o).unwrap().ports, vec![80, 443]);

        o.ports = vec![0];
        assert!(plan(&o).is_err());
    }

    #[test]
    fn plan_enforces_the_port_limit_in_rust() {
        let mut o = opts("10.0.0.1");
        o.ports = (1..=3000u16).collect();
        let err = plan(&o).unwrap_err();
        assert!(err.contains("2048"), "{err}");
    }

    #[test]
    fn plan_rejects_unreasonable_workloads() {
        let mut o = opts("10.0.0.0/16");
        o.ports = (1..=2000u16).collect();
        let err = plan(&o).unwrap_err();
        assert!(err.contains("connection attempts"), "{err}");
        // The message explains the arithmetic instead of silently truncating.
        assert!(err.contains("65,534"), "{err}");
    }

    #[test]
    fn plan_warns_but_allows_large_legitimate_scans() {
        let mut o = opts("10.0.0.0/24");
        o.ports = (1..=2048u16).collect();
        let p = plan(&o).unwrap();
        assert_eq!(p.workload, 254 * 2048);
        assert!(p.warning.is_some());
    }

    #[test]
    fn plan_does_not_warn_about_an_ordinary_lan_sweep() {
        let p = plan(&opts("192.168.1.0/24")).unwrap();
        assert_eq!(p.hosts.len(), 254);
        assert_eq!(p.ports.len(), ports::DEFAULT_PORTS.len());
        assert!(p.warning.is_none());
    }

    #[test]
    fn limits_are_clamped_into_sustainable_ranges() {
        let mut o = opts("10.0.0.1");
        o.concurrency = 100_000;
        o.tcp_concurrency = Some(0);
        o.ping_concurrency = Some(9_999);
        let limits = o.limits();
        assert_eq!(limits.host_concurrency, 1_024);
        assert_eq!(limits.tcp_concurrency, 8);
        assert_eq!(limits.ping_concurrency, 128);
    }

    #[test]
    fn per_host_fanout_stays_bounded_and_useful() {
        let limits = ScanLimits::default();
        // A wide sweep keeps the pending-future count proportional to the
        // global ceiling rather than to the port count.
        assert_eq!(limits.per_host_fanout(254), 8);
        // A single host may use the whole TCP budget.
        assert_eq!(limits.per_host_fanout(1), 256);
    }

    /// Serializes tests that touch the global cancellation state, so one test's
    /// cancel request can never leak into another running in parallel. A tokio
    /// mutex, because the guard is held across awaits.
    fn cancel_test_mutex() -> &'static tokio::sync::Mutex<()> {
        static GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        &GUARD
    }

    /// Register a hook that requests a cancel exactly when the scan reaches the
    /// given checkpoint, making phase-specific cancellation deterministic.
    fn cancel_at(target: Checkpoint) {
        *CHECKPOINT_HOOK.lock().unwrap() = Some(Box::new(move |cp| {
            if cp == target {
                request_cancel();
            }
        }));
    }

    fn clear_checkpoint_hook() {
        *CHECKPOINT_HOOK.lock().unwrap() = None;
    }

    /// Run a short scan of a TEST-NET range, cancelling at `cp`.
    async fn run_cancelled_at(
        cp: Checkpoint,
        target: &str,
        arp_assist: Option<bool>,
    ) -> ScanResult {
        cancel_at(cp);
        let mut o = opts(target);
        o.timeout_ms = 50;
        o.arp_assist = arp_assist;
        let scan_id = next_scan_id();
        let result = run(o, scan_id, None).await.unwrap();
        clear_checkpoint_hook();
        result
    }

    #[test]
    fn cancellation_is_scoped_to_one_scan() {
        let _guard = cancel_test_mutex().blocking_lock();
        ACTIVE_SCAN.store(7, Ordering::Relaxed);
        CANCEL_SCAN.store(0, Ordering::Relaxed);
        assert!(!cancelled(7));
        request_cancel();
        assert!(cancelled(7));
        // A newer scan is unaffected by the older cancel request.
        assert!(!cancelled(8));
        ACTIVE_SCAN.store(0, Ordering::Relaxed);
        CANCEL_SCAN.store(0, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn cancellable_sleep_ends_early_when_cancelled() {
        let _guard = cancel_test_mutex().lock().await;
        ACTIVE_SCAN.store(41, Ordering::Relaxed);
        CANCEL_SCAN.store(0, Ordering::Relaxed);
        let begun = Instant::now();
        let waiter = tokio::spawn(cancellable_sleep(41, Duration::from_secs(30)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        request_cancel();
        assert!(waiter.await.unwrap(), "the wait must report the cancel");
        assert!(
            begun.elapsed() < Duration::from_secs(5),
            "a 30s wait must end promptly after the cancel"
        );
        ACTIVE_SCAN.store(0, Ordering::Relaxed);
        CANCEL_SCAN.store(0, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn cancellable_sleep_runs_to_completion_without_a_cancel() {
        let _guard = cancel_test_mutex().lock().await;
        ACTIVE_SCAN.store(42, Ordering::Relaxed);
        CANCEL_SCAN.store(0, Ordering::Relaxed);
        assert!(!cancellable_sleep(42, Duration::from_millis(20)).await);
        ACTIVE_SCAN.store(0, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn cancel_during_initial_probing_stops_the_sweep() {
        let _guard = cancel_test_mutex().lock().await;
        let result =
            run_cancelled_at(Checkpoint::BeforeProbing, "203.0.113.1-8", Some(false)).await;
        assert!(result.cancelled);
        assert_eq!(result.probed, 0, "no address may be probed after Stop");
        assert_eq!(result.scanned, 8);
    }

    #[tokio::test]
    async fn cancel_during_arp_settle_is_honoured() {
        let _guard = cancel_test_mutex().lock().await;
        let result =
            run_cancelled_at(Checkpoint::BeforeArpSettle, "203.0.113.1-4", Some(false)).await;
        assert!(result.cancelled);
        assert_eq!(result.probed, 4, "probing had already finished");
    }

    #[tokio::test]
    async fn cancel_before_quiet_device_confirmation_skips_the_pass() {
        let _guard = cancel_test_mutex().lock().await;
        // arp_assist on, so the confirmation pass would run for every address
        // lacking an ARP entry — unless the cancel stops it first.
        let begun = Instant::now();
        let result = run_cancelled_at(Checkpoint::BeforeConfirm, "203.0.113.1-4", Some(true)).await;
        assert!(result.cancelled);
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "the confirm pass and second settle must be skipped"
        );
    }

    #[tokio::test]
    async fn cancel_during_the_second_settle_is_honoured() {
        let _guard = cancel_test_mutex().lock().await;
        let result =
            run_cancelled_at(Checkpoint::BeforeSecondSettle, "203.0.113.1-4", Some(true)).await;
        assert!(result.cancelled);
    }

    #[tokio::test]
    async fn cancel_during_hostname_resolution_launches_no_lookups() {
        let _guard = cancel_test_mutex().lock().await;
        // 127.0.0.1 is deterministically alive (a connect to a closed loopback
        // port is refused, which proves liveness), so the scan has a host to
        // resolve when the cancel lands.
        let result = run_cancelled_at(Checkpoint::BeforeDns, "127.0.0.1", Some(false)).await;
        assert!(result.cancelled);
        assert_eq!(result.hosts.len(), 1, "partial hosts must be preserved");
        assert_eq!(result.hosts[0].ip, "127.0.0.1");
        assert!(
            result.hosts[0].hostname.is_none(),
            "no reverse-DNS lookup may be launched after Stop"
        );
    }

    #[tokio::test]
    async fn cancel_immediately_before_completion_is_still_recorded() {
        let _guard = cancel_test_mutex().lock().await;
        // Every phase ran to the end; the final flag must still say cancelled,
        // which proves it is decided at the end rather than captured earlier.
        let result = run_cancelled_at(Checkpoint::BeforeFinish, "203.0.113.1-2", Some(false)).await;
        assert!(result.cancelled);
        assert_eq!(result.probed, 2);
    }

    #[tokio::test]
    async fn a_cancel_from_an_older_scan_does_not_affect_the_next_one() {
        let _guard = cancel_test_mutex().lock().await;
        let first = run_cancelled_at(Checkpoint::BeforeFinish, "203.0.113.1-2", Some(false)).await;
        assert!(first.cancelled);

        // The next scan starts with the stale cancel request still stored and
        // must clear it rather than aborting immediately.
        let mut o = opts("203.0.113.1-2");
        o.timeout_ms = 50;
        o.arp_assist = Some(false);
        let scan_id = next_scan_id();
        let second = run(o, scan_id, None).await.unwrap();
        assert!(!second.cancelled);
        assert_eq!(second.probed, 2);
    }

    #[tokio::test]
    async fn cancelled_scan_returns_partial_results_and_reports_progress() {
        let _guard = cancel_test_mutex().lock().await;
        // 203.0.113.0/24 is the reserved TEST-NET-3 documentation range, so this
        // never reaches a real host; cancelling immediately means almost nothing
        // is probed and the scan must still return a well-formed result.
        let (tx, mut rx) = tokio::sync::mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let mut o = opts("203.0.113.0/24");
        o.timeout_ms = 50;
        o.arp_assist = Some(false);
        let scan_id = next_scan_id();

        let handle = tokio::spawn(async move { run(o, scan_id, Some(tx)).await });
        // Give the sweep a moment to start, then stop it.
        tokio::time::sleep(Duration::from_millis(60)).await;
        request_cancel();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.scan_id, scan_id);
        assert!(result.cancelled);
        assert_eq!(result.scanned, 254);
        assert!(result.probed <= result.scanned);

        let mut saw_started = false;
        let mut saw_cancelled_phase = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                ScanEvent::Started(s) => {
                    saw_started = true;
                    assert_eq!(s.scan_id, scan_id);
                    assert_eq!(s.total, 254);
                }
                ScanEvent::Progress(p) => {
                    assert_eq!(p.scan_id, scan_id);
                    if p.phase == ScanPhase::Cancelled {
                        saw_cancelled_phase = true;
                    }
                }
                _ => {}
            }
        }
        assert!(saw_started, "a scan must announce itself before probing");
        assert!(saw_cancelled_phase, "the final phase must report cancelled");
    }

    #[tokio::test]
    async fn scan_completes_when_the_event_receiver_disappears() {
        let _guard = cancel_test_mutex().lock().await;
        // Simulates the window closing mid-scan: the receiver is dropped before
        // the scan even starts, and every send must fail fast instead of
        // wedging the scanner.
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        drop(rx);
        let mut o = opts("203.0.113.1-4");
        o.timeout_ms = 50;
        o.arp_assist = Some(false);
        let scan_id = next_scan_id();
        let result = run(o, scan_id, Some(tx)).await.unwrap();
        assert!(!result.cancelled);
        assert_eq!(result.probed, 4);
    }

    #[tokio::test]
    async fn cancellation_unblocks_a_scan_stuck_on_a_congested_channel() {
        let _guard = cancel_test_mutex().lock().await;
        // Capacity one and a receiver that never reads: the Started event fills
        // the channel and the first discovery blocks on it. Stop must free the
        // scan; the returned result is the source of truth for what was found.
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let mut o = opts("127.0.0.1");
        o.timeout_ms = 200;
        o.arp_assist = Some(false);
        let scan_id = next_scan_id();
        let handle = tokio::spawn(async move { run(o, scan_id, Some(tx)).await });

        // Wait until this scan owns the cancellation slot, then cancel.
        let begun = Instant::now();
        while ACTIVE_SCAN.load(Ordering::Relaxed) != scan_id {
            assert!(
                begun.elapsed() < Duration::from_secs(10),
                "scan never started"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        request_cancel();

        let result = timeout(Duration::from_secs(15), handle)
            .await
            .expect("a cancelled scan must not deadlock on a full event channel")
            .unwrap()
            .unwrap();
        assert!(result.cancelled);
        assert_eq!(result.hosts.len(), 1, "the discovered host is preserved");
        drop(rx);
    }

    #[tokio::test]
    async fn slow_event_consumer_still_receives_every_critical_event() {
        let _guard = cancel_test_mutex().lock().await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut o = opts("127.0.0.1");
        o.timeout_ms = 200;
        o.arp_assist = Some(false);
        let scan_id = next_scan_id();
        let handle = tokio::spawn(async move { run(o, scan_id, Some(tx)).await });

        let mut discovered = 0;
        let mut updated = 0;
        let mut final_phase = None;
        while let Some(event) = rx.recv().await {
            // Draining slowly forces the scanner through the backpressure path.
            tokio::time::sleep(Duration::from_millis(5)).await;
            match event {
                ScanEvent::HostDiscovered { .. } => discovered += 1,
                ScanEvent::HostUpdated { .. } => updated += 1,
                ScanEvent::Progress(p) => final_phase = Some(p.phase),
                _ => {}
            }
        }
        let result = handle.await.unwrap().unwrap();
        assert_eq!(discovered, 1);
        assert_eq!(updated, 1, "the final enrichment update is critical");
        assert_eq!(final_phase, Some(ScanPhase::Done));
        assert_eq!(result.hosts.len(), 1);
    }

    #[tokio::test]
    async fn invalid_target_fails_before_any_probe() {
        let scan_id = next_scan_id();
        let err = run(opts("not-an-ip"), scan_id, None).await.unwrap_err();
        assert!(err.contains("not a valid IPv4 address"), "{err}");
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(65_534), "65,534");
        assert_eq!(thousands(4_000_000), "4,000,000");
    }

    #[test]
    fn normalizes_mac_separators() {
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-ff").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(normalize_mac("aabbccddeeff"), None);
        assert_eq!(normalize_mac("ff:ff:ff:ff:ff:ff"), None);
    }
}
