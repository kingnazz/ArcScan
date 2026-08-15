//! Local service discovery: mDNS and SSDP, bounded and local-only.
//!
//! # What this does and does not do
//!
//! ArcScan asks the local link two questions — "what services are here?" over
//! multicast DNS, and "what UPnP devices are here?" over SSDP — listens for a
//! few seconds, optionally reads a description document from a device that
//! offered one, and closes every socket. It does not stay resident, does not
//! answer queries, does not join a group it then has to leave, and does not
//! cache between scans.
//!
//! # One-shot querying
//!
//! Both protocols are spoken from an *ephemeral* port with the reply directed
//! back to it: mDNS through the unicast-response bit (RFC 6762 §5.1's one-shot
//! querier), SSDP because that is simply how M-SEARCH works. Binding port 5353
//! would collide with the Bonjour or Avahi responder already running on the
//! machine and would make ArcScan a participant on the network rather than a
//! visitor. The cost is real and worth stating: a responder that ignores the
//! unicast bit and answers only to the multicast group is not heard.
//!
//! # Locality
//!
//! [`eligibility`] is the single gate. Discovery runs only when the scan's
//! target lies inside a subnet this computer is actually attached to, and the
//! multicast TTL is 1, so nothing leaves the local link even if a router would
//! have forwarded it. Remote-subnet scans, routed targets and public targets
//! never send a multicast packet.
//!
//! # Bounds
//!
//! Every loop below is bounded by both a deadline and a count, and both are
//! checked before allocating. The limits are collected at the top of this file
//! so the worst case a hostile or broken network can cause is readable in one
//! place.
//!
//! # Cancellation
//!
//! Stop is honoured before each socket is opened, before each query is sent,
//! while waiting for every response, before each description fetch, and before
//! the results are merged. Everything already parsed is kept — a partial pass
//! yields partial knowledge — but the scan is marked interrupted, which is what
//! stops it from producing change events.

pub mod classify;
pub mod http;
pub mod mdns;
pub mod model;
pub mod names;
pub mod ssdp;
pub mod urlguard;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::scanner;

pub use classify::{classify, Classification, ClassifyFacts};
pub use model::{
    Confidence, DeviceType, DiscoveredDevice, DiscoveryReport, DiscoverySource, Evidence,
    EvidenceKind,
};

// --- Hard limits ----------------------------------------------------------
//
// Nothing below grows with the size of the network being scanned. A /24 and a
// /16 cost the same discovery pass, because discovery is a conversation with
// the link, not a sweep of addresses.

/// How long the mDNS conversation may last in total.
pub const MDNS_BUDGET: Duration = Duration::from_millis(2_600);
/// How long the SSDP conversation may last in total.
pub const SSDP_BUDGET: Duration = Duration::from_millis(2_800);
/// How long all description fetches together may take.
pub const DESCRIPTION_BUDGET: Duration = Duration::from_millis(4_000);

/// Most service types followed up after enumeration.
pub const MAX_SERVICE_TYPES: usize = 40;
/// Most service instances resolved.
pub const MAX_INSTANCES: usize = 120;
/// Most mDNS packets read in one pass.
pub const MAX_MDNS_PACKETS: usize = 400;
/// Most SSDP responses read in one pass.
pub const MAX_SSDP_RESPONSES: usize = 300;
/// Most description documents fetched in one scan.
pub const MAX_DESCRIPTION_FETCHES: usize = 24;
/// Description fetches in flight at once.
pub const DESCRIPTION_CONCURRENCY: usize = 4;
/// Most distinct addresses discovery will report on.
pub const MAX_DISCOVERED_DEVICES: usize = 512;
/// Most evidence rows kept for one device from one pass.
pub const MAX_EVIDENCE_PER_DEVICE: usize = 48;

const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SSDP_PORT: u16 = 1900;

/// Which parts of discovery the operator has switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryOptions {
    /// The master switch. Off means no multicast packet is ever sent.
    #[serde(default = "on")]
    pub enabled: bool,
    #[serde(default = "on")]
    pub mdns: bool,
    #[serde(default = "on")]
    pub ssdp: bool,
    /// Whether a device's advertised description URL may be read.
    #[serde(default = "on")]
    pub descriptions: bool,
}

fn on() -> bool {
    true
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        DiscoveryOptions {
            enabled: true,
            mdns: true,
            ssdp: true,
            descriptions: true,
        }
    }
}

/// Everything discovery needs to know about the scan it belongs to.
#[derive(Debug, Clone)]
pub struct DiscoveryContext {
    pub scan_id: u64,
    /// The local subnet containing the target, and this machine's address on
    /// it. `None` when the target is not on a network this computer is on.
    pub local_network: Option<(Ipv4Addr, u8)>,
    pub interface_ip: Option<Ipv4Addr>,
    /// `Some(false)` is the Remote subnet profile, which opts out of every
    /// local assumption including this one.
    pub arp_assist: Option<bool>,
    pub options: DiscoveryOptions,
}

/// Whether discovery may run, and if not, why not in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    Run {
        interface: Ipv4Addr,
        network: (Ipv4Addr, u8),
    },
    Skip(String),
}

/// The single locality gate.
///
/// Discovery is allowed only when *all* of the following hold:
///
/// 1. the operator has not switched it off;
/// 2. the scan is not using the Remote subnet strategy, which declares up front
///    that the target is somewhere else;
/// 3. the target overlaps a subnet this computer is attached to; and
/// 4. that subnet has a usable interface address to send from.
///
/// Anything else is a skip with a reason the History view can show. There is no
/// "probably local" case: a multicast query sent to the wrong link is both
/// useless and a surprise, so uncertainty resolves to not sending.
pub fn eligibility(ctx: &DiscoveryContext) -> Eligibility {
    if !ctx.options.enabled {
        return Eligibility::Skip("Local discovery is switched off in Settings".into());
    }
    if !ctx.options.mdns && !ctx.options.ssdp {
        return Eligibility::Skip("Both discovery protocols are switched off in Settings".into());
    }
    if ctx.arp_assist == Some(false) {
        return Eligibility::Skip(
            "Remote subnet scans skip local discovery, because multicast does not reach them"
                .into(),
        );
    }
    let Some(network) = ctx.local_network else {
        return Eligibility::Skip(
            "This target is not on a network this computer is connected to, so local discovery \
             was skipped"
                .into(),
        );
    };
    let Some(interface) = ctx.interface_ip else {
        return Eligibility::Skip(
            "No local interface address was available to send discovery queries from".into(),
        );
    };
    Eligibility::Run { interface, network }
}

/// What one discovery pass produced.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryOutcome {
    pub report: DiscoveryReport,
    /// Evidence keyed by the address it belongs to.
    pub devices: HashMap<Ipv4Addr, DiscoveredDevice>,
}

impl DiscoveryOutcome {
    fn skipped(reason: String) -> Self {
        DiscoveryOutcome {
            report: DiscoveryReport::skipped(reason),
            devices: HashMap::new(),
        }
    }
}

/// Run a full discovery pass.
///
/// Both protocols run at once, on separate sockets, because they are
/// independent conversations and running them in series would double the wall
/// clock for no extra information.
pub async fn run(ctx: &DiscoveryContext) -> DiscoveryOutcome {
    let (interface, network) = match eligibility(ctx) {
        Eligibility::Skip(reason) => return DiscoveryOutcome::skipped(reason),
        Eligibility::Run { interface, network } => (interface, network),
    };
    // Checked before a socket exists, so Stop pressed during the ARP settle
    // costs nothing here.
    if scanner::is_cancelled(ctx.scan_id) {
        return DiscoveryOutcome::skipped("The scan was stopped before discovery began".into());
    }

    let started = Instant::now();
    let policy = urlguard::LocalPolicy::from_networks(&[network]);

    let mdns_task = async {
        if ctx.options.mdns {
            Some(run_mdns(ctx.scan_id, interface).await)
        } else {
            None
        }
    };
    let ssdp_task = async {
        if ctx.options.ssdp {
            Some(run_ssdp(ctx.scan_id, interface).await)
        } else {
            None
        }
    };
    let (mdns_result, ssdp_result) = futures::join!(mdns_task, ssdp_task);

    let mut devices: HashMap<Ipv4Addr, DiscoveredDevice> = HashMap::new();
    let mut report = DiscoveryReport {
        mdns_attempted: mdns_result.is_some(),
        ssdp_attempted: ssdp_result.is_some(),
        ..Default::default()
    };

    if let Some(harvest) = &mdns_result {
        report.mdns_responses = harvest.packets;
        merge_mdns(harvest, &policy, &mut devices);
    }

    let mut fetch_targets: Vec<(Ipv4Addr, String)> = Vec::new();
    if let Some(harvest) = &ssdp_result {
        report.ssdp_responses = harvest.responses.len();
        fetch_targets = merge_ssdp(harvest, &mut devices);
    }

    // Descriptions come last: they are the only part that opens a connection,
    // and the SSDP headers alone already carry a manufacturer and a type.
    if ctx.options.descriptions && !fetch_targets.is_empty() && !scanner::is_cancelled(ctx.scan_id)
    {
        let outcome = fetch_descriptions(ctx.scan_id, &policy, fetch_targets, &mut devices).await;
        report.descriptions_fetched = outcome.fetched;
        report.descriptions_rejected = outcome.rejected;
        report.description_notes = outcome.notes;
    }

    // Trim and sort so the record is deterministic and bounded regardless of
    // what the network did.
    let mut trimmed: HashMap<Ipv4Addr, DiscoveredDevice> = HashMap::new();
    let mut addresses: Vec<Ipv4Addr> = devices.keys().copied().collect();
    addresses.sort_by_key(|ip| u32::from(*ip));
    for ip in addresses.into_iter().take(MAX_DISCOVERED_DEVICES) {
        let Some(mut device) = devices.remove(&ip) else {
            continue;
        };
        if device.is_empty() {
            continue;
        }
        device.sort();
        device.evidence.truncate(MAX_EVIDENCE_PER_DEVICE);
        trimmed.insert(ip, device);
    }

    report.devices_enriched = trimmed.len();
    report.duration_ms = started.elapsed().as_millis() as u64;
    report.interrupted = scanner::is_cancelled(ctx.scan_id);

    DiscoveryOutcome {
        report,
        devices: trimmed,
    }
}

// --- mDNS -----------------------------------------------------------------

/// Records read from the link, each paired with the address that sent them.
#[derive(Debug, Default)]
pub struct MdnsHarvest {
    pub records: Vec<(Ipv4Addr, mdns::Record)>,
    pub packets: usize,
}

async fn run_mdns(scan_id: u64, interface: Ipv4Addr) -> MdnsHarvest {
    let mut harvest = MdnsHarvest::default();
    let deadline = Instant::now() + MDNS_BUDGET;

    let Some(socket) = open_socket(interface).await else {
        return harvest;
    };
    let group = SocketAddr::V4(SocketAddrV4::new(MDNS_GROUP, MDNS_PORT));

    // 1. Ask the link which service types exist. One packet, and the answer
    //    replaces any hardcoded list this scan would otherwise have to send.
    if scanner::is_cancelled(scan_id) {
        return harvest;
    }
    let _ = socket
        .send_to(&mdns::build_query(&[mdns::SERVICE_ENUMERATION]), group)
        .await;

    let enumeration_until = (Instant::now() + Duration::from_millis(900)).min(deadline);
    collect_mdns(&socket, scan_id, enumeration_until, &mut harvest).await;

    // 2. Follow up on what it named. A network that answered nothing gets the
    //    short fallback list instead — one packet either way.
    let mut types: Vec<String> = harvest
        .records
        .iter()
        .filter(|(_, r)| r.name.trim_end_matches('.') == mdns::SERVICE_ENUMERATION)
        .filter_map(|(_, r)| match &r.data {
            mdns::RecordData::Ptr(target) => Some(target.trim_end_matches('.').to_string()),
            _ => None,
        })
        .collect();
    types.sort();
    types.dedup();
    types.truncate(MAX_SERVICE_TYPES);

    let fallback: Vec<String>;
    let query_names: Vec<&str> = if types.is_empty() {
        fallback = mdns::FALLBACK_SERVICES
            .iter()
            .map(|s| s.to_string())
            .collect();
        fallback.iter().map(String::as_str).collect()
    } else {
        types.iter().map(String::as_str).collect()
    };

    if scanner::is_cancelled(scan_id) || Instant::now() >= deadline {
        return harvest;
    }
    // Split into modest packets rather than one enormous question section.
    for chunk in query_names.chunks(12) {
        if scanner::is_cancelled(scan_id) {
            return harvest;
        }
        let _ = socket.send_to(&mdns::build_query(chunk), group).await;
    }

    let browse_until = (Instant::now() + Duration::from_millis(1_000)).min(deadline);
    collect_mdns(&socket, scan_id, browse_until, &mut harvest).await;

    // 3. Resolve the instances that were named but not described. Most
    //    responders already included SRV, TXT and A in the additional section,
    //    so this is usually a very short list.
    let described: HashSet<String> = harvest
        .records
        .iter()
        .filter(|(_, r)| r.rtype == mdns::TYPE_SRV)
        .map(|(_, r)| r.name.trim_end_matches('.').to_lowercase())
        .collect();
    let mut pending: Vec<String> = harvest
        .records
        .iter()
        .filter_map(|(_, r)| match &r.data {
            mdns::RecordData::Ptr(target) if r.name.starts_with('_') => {
                Some(target.trim_end_matches('.').to_string())
            }
            _ => None,
        })
        .filter(|instance| !described.contains(&instance.to_lowercase()))
        .collect();
    pending.sort();
    pending.dedup();
    pending.truncate(MAX_INSTANCES);

    if !pending.is_empty() && !scanner::is_cancelled(scan_id) && Instant::now() < deadline {
        for chunk in pending.chunks(10) {
            if scanner::is_cancelled(scan_id) {
                return harvest;
            }
            let _ = socket
                .send_to(&mdns::build_instance_query(chunk), group)
                .await;
        }
        collect_mdns(&socket, scan_id, deadline, &mut harvest).await;
    }

    harvest
}

async fn collect_mdns(
    socket: &UdpSocket,
    scan_id: u64,
    deadline: Instant,
    harvest: &mut MdnsHarvest,
) {
    let mut buf = vec![0u8; mdns::MAX_PACKET_BYTES];
    while harvest.packets < MAX_MDNS_PACKETS {
        let Some((len, from)) = recv_until(socket, scan_id, deadline, &mut buf).await else {
            return;
        };
        let IpAddr::V4(source) = from.ip() else {
            continue;
        };
        harvest.packets += 1;
        let Some(message) = mdns::parse(&buf[..len]) else {
            continue;
        };
        for record in message.answers {
            if harvest.records.len() >= MAX_MDNS_PACKETS * 8 {
                return;
            }
            harvest.records.push((source, record));
        }
    }
}

/// Turn harvested records into per-address evidence.
///
/// Address binding, in order: the A record for a service's SRV target when that
/// address is inside the scanned network, otherwise the address the packet came
/// from. A name with no address anywhere never creates a device — an instance
/// label is evidence about a device, never a device in its own right.
pub fn merge_mdns(
    harvest: &MdnsHarvest,
    policy: &urlguard::LocalPolicy,
    devices: &mut HashMap<Ipv4Addr, DiscoveredDevice>,
) {
    let mut a_records: HashMap<String, Ipv4Addr> = HashMap::new();
    let mut aaaa_records: HashMap<String, BTreeSet<std::net::Ipv6Addr>> = HashMap::new();
    let mut srv: HashMap<String, (String, u16)> = HashMap::new();
    let mut txt: HashMap<String, std::collections::BTreeMap<String, String>> = HashMap::new();
    let mut instances: BTreeSet<String> = BTreeSet::new();
    let mut sources: HashMap<String, Ipv4Addr> = HashMap::new();

    for (from, record) in &harvest.records {
        let name = record.name.trim_end_matches('.').to_string();
        let key = name.to_lowercase();
        match &record.data {
            mdns::RecordData::A(ip) => {
                if policy.allows(*ip) {
                    a_records.entry(key).or_insert(*ip);
                }
            }
            mdns::RecordData::Aaaa(ip) => {
                aaaa_records.entry(key).or_default().insert(*ip);
            }
            mdns::RecordData::Srv { port, target } => {
                srv.entry(key.clone())
                    .or_insert((target.trim_end_matches('.').to_string(), *port));
                instances.insert(name);
                sources.entry(key).or_insert(*from);
            }
            mdns::RecordData::Txt(map) => {
                txt.entry(key.clone()).or_insert_with(|| map.clone());
                sources.entry(key).or_insert(*from);
            }
            mdns::RecordData::Ptr(target) => {
                let target = target.trim_end_matches('.').to_string();
                if name.starts_with('_') && name != mdns::SERVICE_ENUMERATION {
                    instances.insert(target.clone());
                    sources.entry(target.to_lowercase()).or_insert(*from);
                }
            }
            mdns::RecordData::Other => {}
        }
    }

    for instance in instances {
        let key = instance.to_lowercase();
        let Some((label, service_type)) = mdns::split_instance(&instance) else {
            continue;
        };
        let target = srv.get(&key).map(|(t, _)| t.to_lowercase());
        let address = target
            .as_ref()
            .and_then(|t| a_records.get(t).copied())
            .filter(|ip| policy.allows(*ip))
            .or_else(|| sources.get(&key).copied().filter(|ip| policy.allows(*ip)));
        let Some(address) = address else {
            // No address: nothing to attach the evidence to, and inventing a
            // device from a name is exactly what this release must not do.
            continue;
        };

        let entry = devices
            .entry(address)
            .or_insert_with(|| DiscoveredDevice::new(address));

        let service = service_type.trim_end_matches(".local").to_string();
        entry.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Service,
            &service,
            &service,
            Confidence::High,
        ));
        if let Some((_, port)) = srv.get(&key) {
            entry.add(
                Evidence::new(
                    DiscoverySource::Mdns,
                    EvidenceKind::ServicePort,
                    &service,
                    port.to_string(),
                    Confidence::High,
                )
                .with_meta("service", &service),
            );
        }

        // The instance label is the name a person gave the device, when it is
        // not simply the service type repeated back.
        if let Some(name) = names::tidy_name(&label) {
            let confidence = if names::is_generic_name(&name) {
                Confidence::Low
            } else {
                Confidence::High
            };
            entry.add(
                Evidence::new(
                    DiscoverySource::Mdns,
                    EvidenceKind::DisplayName,
                    "",
                    name,
                    confidence,
                )
                .with_meta("service", &service),
            );
        }
        // The instance name is recorded for continuity only. It is never an
        // identity key: a device that renames itself is the same device, and
        // two devices can advertise the same instance label.
        entry.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::ProtocolIdentifier,
            "mdns_instance",
            &instance,
            Confidence::Medium,
        ));

        if let Some(target) = &target {
            if let Some(host) = names::tidy_name(mdns::strip_local(target)) {
                entry.add(Evidence::new(
                    DiscoverySource::Mdns,
                    EvidenceKind::Hostname,
                    "",
                    host,
                    Confidence::Medium,
                ));
            }
            for ip in aaaa_records.get(target).into_iter().flatten() {
                entry.ipv6.insert(*ip);
                entry.add(Evidence::new(
                    DiscoverySource::Mdns,
                    EvidenceKind::Ipv6Address,
                    "",
                    ip.to_string(),
                    Confidence::Medium,
                ));
            }
        }

        if let Some(properties) = txt.get(&key) {
            apply_txt(entry, properties);
        }
    }
}

/// Pull the few TXT keys that are worth keeping.
///
/// Deliberately a short allow-list rather than the whole record: a TXT block is
/// full of protocol plumbing (`rp`, `pdl`, `txtvers`, printer feature flags)
/// that means nothing to a person and would bloat the database. The keys below
/// are the ones vendors actually use to name the hardware.
fn apply_txt(
    entry: &mut DiscoveredDevice,
    properties: &std::collections::BTreeMap<String, String>,
) {
    let take = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| properties.get(*k))
            .and_then(|v| model::sanitize_field(v))
    };

    if let Some(model_name) = take(&["ty", "model", "md", "usb_mdl", "product"]) {
        entry.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Model,
            "",
            model_name,
            Confidence::Medium,
        ));
    }
    if let Some(manufacturer) = take(&["usb_mfg", "mfg", "manufacturer", "vendor"]) {
        entry.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Manufacturer,
            "",
            manufacturer,
            Confidence::Medium,
        ));
    }
    // Apple publishes its hardware model here (`MacBookPro18,3`, `AppleTV6,2`),
    // which is the single most useful TXT value on a home network.
    if let Some(hardware) = take(&["am", "model"]) {
        entry.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::ModelNumber,
            "",
            hardware,
            Confidence::Medium,
        ));
    }
}

// --- SSDP -----------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SsdpHarvest {
    /// Responses paired with the address they came from.
    pub responses: Vec<(Ipv4Addr, ssdp::Response)>,
}

async fn run_ssdp(scan_id: u64, interface: Ipv4Addr) -> SsdpHarvest {
    let mut harvest = SsdpHarvest::default();
    let deadline = Instant::now() + SSDP_BUDGET;

    let Some(socket) = open_socket(interface).await else {
        return harvest;
    };
    let group = SocketAddr::V4(SocketAddrV4::new(SSDP_GROUP, SSDP_PORT));

    for target in ssdp::SEARCH_TARGETS {
        if scanner::is_cancelled(scan_id) {
            return harvest;
        }
        let _ = socket.send_to(&ssdp::build_msearch(target), group).await;
    }

    let mut buf = vec![0u8; ssdp::MAX_RESPONSE_BYTES];
    let mut seen: HashSet<(Ipv4Addr, String)> = HashSet::new();
    while harvest.responses.len() < MAX_SSDP_RESPONSES {
        let Some((len, from)) = recv_until(&socket, scan_id, deadline, &mut buf).await else {
            break;
        };
        let IpAddr::V4(source) = from.ip() else {
            continue;
        };
        let Some(response) = ssdp::parse(&buf[..len]) else {
            continue;
        };
        // One device answers `ssdp:all` once per service, which is dozens of
        // near-identical datagrams. Keyed on the USN so each distinct thing is
        // counted once and a chatty device cannot fill the budget.
        let key = (source, response.usn().unwrap_or_default().to_string());
        if !seen.insert(key) {
            continue;
        }
        harvest.responses.push((source, response));
    }

    harvest
}

/// Fold SSDP responses into evidence, and return the description URLs worth
/// fetching, de-duplicated so one URL is never fetched twice in a scan.
pub fn merge_ssdp(
    harvest: &SsdpHarvest,
    devices: &mut HashMap<Ipv4Addr, DiscoveredDevice>,
) -> Vec<(Ipv4Addr, String)> {
    let mut fetches: Vec<(Ipv4Addr, String)> = Vec::new();
    let mut queued: HashSet<String> = HashSet::new();

    for (source, response) in &harvest.responses {
        let entry = devices
            .entry(*source)
            .or_insert_with(|| DiscoveredDevice::new(*source));

        // The device type declared in ST or USN. This is a device describing
        // itself in a protocol built for the purpose, so it carries real weight.
        for value in [response.search_target(), response.usn()]
            .into_iter()
            .flatten()
        {
            if let Some(kind) = ssdp::urn_device_type(value) {
                entry.add(Evidence::new(
                    DiscoverySource::Ssdp,
                    EvidenceKind::DeviceType,
                    "upnp",
                    kind,
                    Confidence::High,
                ));
            }
        }
        if let Some(udn) = response.udn() {
            // Continuity evidence within one network only. Never an identity
            // key, and never compared across network scopes.
            entry.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::ProtocolIdentifier,
                "upnp_udn",
                udn,
                Confidence::Medium,
            ));
        }
        // The SERVER header is a free-text banner. It is recorded at low
        // confidence and never used to name a device, because "Linux/4.4
        // UPnP/1.0" describes a million devices.
        if let Some(server) = response.server().and_then(model::sanitize_field) {
            entry.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::Manufacturer,
                "server_banner",
                server,
                Confidence::Low,
            ));
        }

        if let Some(location) = response.location() {
            let location = location.to_string();
            if queued.len() < MAX_DESCRIPTION_FETCHES && queued.insert(location.clone()) {
                fetches.push((*source, location));
            }
        }
    }
    fetches
}

/// What the description pass managed to do.
#[derive(Debug, Default)]
struct DescriptionOutcome {
    fetched: usize,
    rejected: usize,
    notes: Vec<String>,
}

impl DescriptionOutcome {
    /// Record a reason, de-duplicated and capped. History shows these, and a
    /// network with fifty devices all refusing the same way should read as one
    /// line, not fifty.
    fn note(&mut self, reason: String) {
        if self.notes.len() < 8 && !self.notes.contains(&reason) {
            self.notes.push(reason);
        }
    }
}

/// Validate and read the queued description documents.
///
/// Every URL goes through [`urlguard`] first; anything it refuses is counted
/// and dropped *without a connection being made*, which is the whole point —
/// the refusal has to happen before a socket exists, not after.
async fn fetch_descriptions(
    scan_id: u64,
    policy: &urlguard::LocalPolicy,
    targets: Vec<(Ipv4Addr, String)>,
    devices: &mut HashMap<Ipv4Addr, DiscoveredDevice>,
) -> DescriptionOutcome {
    let deadline = Instant::now() + DESCRIPTION_BUDGET;
    let mut outcome = DescriptionOutcome::default();
    let mut approved: Vec<(Ipv4Addr, urlguard::ValidatedUrl)> = Vec::new();

    for (source, raw) in targets {
        let parsed = match urlguard::parse_location(&raw) {
            Ok(parsed) => parsed,
            Err(rejection) => {
                outcome.rejected += 1;
                outcome.note(format!(
                    "A description address was refused: {}",
                    rejection.reason()
                ));
                continue;
            }
        };
        // A literal needs no resolution, which is both faster and one fewer
        // thing that can change under us.
        let resolved: Vec<IpAddr> = if parsed.is_literal() {
            Vec::new()
        } else {
            resolve_host(&parsed.host, parsed.port).await
        };
        match urlguard::authorize(&parsed, &resolved, policy) {
            Ok(url) => approved.push((source, url)),
            Err(rejection) => {
                outcome.rejected += 1;
                outcome.note(format!(
                    "A description address was refused: {}",
                    rejection.reason()
                ));
            }
        }
    }

    if approved.is_empty() || scanner::is_cancelled(scan_id) {
        return outcome;
    }

    type FetchResult = (Ipv4Addr, Result<xml::Description, String>);
    let results: Vec<FetchResult> = stream::iter(approved)
        .map(|(source, url)| async move {
            if scanner::is_cancelled(scan_id) || Instant::now() >= deadline {
                return (source, Err(String::new()));
            }
            let document = match http::fetch_description(&url).await {
                Ok(body) => body,
                Err(error) => {
                    return (
                        source,
                        Err(format!(
                            "A description could not be read: {}",
                            error.reason()
                        )),
                    )
                }
            };
            match xml::parse(&document) {
                Ok(description) => (source, Ok(description)),
                Err(error) => (
                    source,
                    Err(format!("A description was not usable: {}", error.reason())),
                ),
            }
        })
        .buffer_unordered(DESCRIPTION_CONCURRENCY)
        .collect()
        .await;

    for (source, result) in results {
        match result {
            Ok(description) if !description.is_empty() => {
                outcome.fetched += 1;
                let entry = devices
                    .entry(source)
                    .or_insert_with(|| DiscoveredDevice::new(source));
                apply_description(entry, &description);
            }
            Ok(_) => {}
            Err(note) if note.is_empty() => {}
            Err(note) => outcome.note(note),
        }
    }
    outcome
}

/// Attach a parsed description to a device.
pub fn apply_description(entry: &mut DiscoveredDevice, description: &xml::Description) {
    if let Some(name) = description
        .friendly_name
        .as_deref()
        .and_then(names::tidy_name)
    {
        let confidence = if names::is_generic_name(&name) {
            Confidence::Low
        } else {
            Confidence::High
        };
        entry.add(Evidence::new(
            DiscoverySource::Ssdp,
            EvidenceKind::DisplayName,
            "friendly_name",
            name,
            confidence,
        ));
    }
    for (kind, value, confidence) in [
        (
            EvidenceKind::Manufacturer,
            description.manufacturer.clone(),
            Confidence::High,
        ),
        (
            EvidenceKind::Model,
            description.model_name.clone(),
            Confidence::High,
        ),
        (
            EvidenceKind::ModelNumber,
            description.model_number.clone(),
            Confidence::High,
        ),
        (
            EvidenceKind::SerialNumber,
            description.serial_number.clone(),
            Confidence::Medium,
        ),
    ] {
        if let Some(value) = value {
            entry.add(Evidence::new(
                DiscoverySource::Ssdp,
                kind,
                "",
                value,
                confidence,
            ));
        }
    }
    for kind in description
        .device_type
        .iter()
        .chain(description.embedded_types.iter())
    {
        if let Some(bare) = ssdp::urn_device_type(kind) {
            entry.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::DeviceType,
                "upnp",
                bare,
                Confidence::High,
            ));
        }
    }
    for service in &description.services {
        if let Some(bare) = service.rsplit(':').nth(1) {
            entry.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::Service,
                bare,
                bare,
                Confidence::Medium,
            ));
        }
    }
    // Recorded so the drawer can show where a device's own page lives. ArcScan
    // never opens it: the operator does, through the existing Web action, which
    // validates the address itself.
    if let Some(url) = description
        .presentation_url
        .as_deref()
        .and_then(model::sanitize_field)
    {
        entry.add(Evidence::new(
            DiscoverySource::Ssdp,
            EvidenceKind::Url,
            "presentation",
            url,
            Confidence::Medium,
        ));
    }
}

pub mod xml;

// --- Shared socket helpers ------------------------------------------------

/// Open an ephemeral UDP socket bound to the interface the scan is using.
///
/// TTL 1 keeps every query on the local link: even a router configured to
/// forward multicast will not carry it, which is what makes "local network
/// only" a property of the packet rather than a promise about configuration.
/// Multicast loopback is left on so a service running on this same machine is
/// discovered like any other.
async fn open_socket(interface: Ipv4Addr) -> Option<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(interface, 0)))
        .await
        .ok()?;
    // A failure here is not fatal: the defaults are still local-ish, and a
    // platform that refuses the option should not lose discovery entirely.
    let _ = socket.set_multicast_ttl_v4(1);
    let _ = socket.set_multicast_loop_v4(true);
    Some(socket)
}

/// Receive one datagram, giving up at the deadline or the moment Stop is
/// pressed — whichever comes first.
async fn recv_until(
    socket: &UdpSocket,
    scan_id: u64,
    deadline: Instant,
    buf: &mut [u8],
) -> Option<(usize, SocketAddr)> {
    let now = Instant::now();
    if now >= deadline || scanner::is_cancelled(scan_id) {
        return None;
    }
    let remaining = deadline - now;
    tokio::select! {
        biased;
        _ = scanner::cancel_requested(scan_id) => None,
        result = tokio::time::timeout(remaining, socket.recv_from(buf)) => match result {
            Ok(Ok(pair)) => Some(pair),
            _ => None,
        },
    }
}

/// Resolve a host name for a description URL, on a short leash.
async fn resolve_host(host: &str, port: u16) -> Vec<IpAddr> {
    let query = format!("{host}:{port}");
    match tokio::time::timeout(Duration::from_millis(600), tokio::net::lookup_host(query)).await {
        Ok(Ok(addrs)) => addrs.map(|a| a.ip()).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> DiscoveryContext {
        DiscoveryContext {
            scan_id: 1,
            local_network: Some(("192.0.2.0".parse().unwrap(), 24)),
            interface_ip: Some("192.0.2.2".parse().unwrap()),
            arp_assist: None,
            options: DiscoveryOptions::default(),
        }
    }

    fn skip_reason(ctx: &DiscoveryContext) -> String {
        match eligibility(ctx) {
            Eligibility::Skip(reason) => reason,
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    #[test]
    fn a_local_scan_with_an_interface_is_eligible() {
        assert_eq!(
            eligibility(&context()),
            Eligibility::Run {
                interface: "192.0.2.2".parse().unwrap(),
                network: ("192.0.2.0".parse().unwrap(), 24),
            }
        );
    }

    #[test]
    fn a_target_on_no_local_network_is_skipped() {
        let ctx = DiscoveryContext {
            local_network: None,
            ..context()
        };
        assert!(skip_reason(&ctx).contains("not on a network this computer is connected to"));
    }

    #[test]
    fn the_remote_subnet_profile_skips_discovery_outright() {
        let ctx = DiscoveryContext {
            arp_assist: Some(false),
            ..context()
        };
        assert!(skip_reason(&ctx).contains("Remote subnet"));
    }

    #[test]
    fn the_master_switch_and_both_protocol_switches_are_honoured() {
        let off = DiscoveryContext {
            options: DiscoveryOptions {
                enabled: false,
                ..Default::default()
            },
            ..context()
        };
        assert!(skip_reason(&off).contains("switched off"));

        let neither = DiscoveryContext {
            options: DiscoveryOptions {
                mdns: false,
                ssdp: false,
                ..Default::default()
            },
            ..context()
        };
        assert!(skip_reason(&neither).contains("Both discovery protocols"));

        // One protocol on is still eligible.
        let one = DiscoveryContext {
            options: DiscoveryOptions {
                ssdp: false,
                ..Default::default()
            },
            ..context()
        };
        assert!(matches!(eligibility(&one), Eligibility::Run { .. }));
    }

    #[test]
    fn a_local_network_with_no_interface_address_is_skipped() {
        let ctx = DiscoveryContext {
            interface_ip: None,
            ..context()
        };
        assert!(skip_reason(&ctx).contains("No local interface address"));
    }

    #[test]
    fn every_skip_reason_is_a_sentence_a_person_can_read() {
        for ctx in [
            DiscoveryContext {
                local_network: None,
                ..context()
            },
            DiscoveryContext {
                arp_assist: Some(false),
                ..context()
            },
            DiscoveryContext {
                interface_ip: None,
                ..context()
            },
        ] {
            let reason = skip_reason(&ctx);
            assert!(reason.len() > 20, "{reason}");
            assert!(reason.chars().next().unwrap().is_uppercase(), "{reason}");
        }
    }

    #[test]
    fn discovery_options_default_to_everything_on() {
        let d = DiscoveryOptions::default();
        assert!(d.enabled && d.mdns && d.ssdp && d.descriptions);
        // Absent fields in a stored preference read as on, so a scan saved by
        // 1.8.1 and replayed here does not silently lose discovery.
        let parsed: DiscoveryOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn the_time_budget_stays_within_a_few_seconds() {
        // Both protocols run at once, so the wall clock is the larger of the
        // two plus the description window.
        let worst = MDNS_BUDGET.max(SSDP_BUDGET) + DESCRIPTION_BUDGET;
        assert!(worst <= Duration::from_secs(8), "{worst:?}");
        assert!(MDNS_BUDGET <= Duration::from_secs(4));
        assert!(SSDP_BUDGET <= Duration::from_secs(4));
    }

    // --- Merging, exercised through the real parsers -----------------------

    fn policy() -> urlguard::LocalPolicy {
        urlguard::LocalPolicy::from_networks(&[("192.0.2.0".parse().unwrap(), 24)])
    }

    fn record(name: &str, rtype: u16, data: mdns::RecordData) -> mdns::Record {
        mdns::Record {
            name: name.into(),
            rtype,
            ttl: 120,
            data,
        }
    }

    fn printer_harvest(from: &str) -> MdnsHarvest {
        let from: Ipv4Addr = from.parse().unwrap();
        MdnsHarvest {
            packets: 1,
            records: vec![
                (
                    from,
                    record(
                        "_ipp._tcp.local",
                        mdns::TYPE_PTR,
                        mdns::RecordData::Ptr("Studio Printer._ipp._tcp.local".into()),
                    ),
                ),
                (
                    from,
                    record(
                        "Studio Printer._ipp._tcp.local",
                        mdns::TYPE_SRV,
                        mdns::RecordData::Srv {
                            port: 631,
                            target: "studio-printer.local".into(),
                        },
                    ),
                ),
                (
                    from,
                    record(
                        "Studio Printer._ipp._tcp.local",
                        mdns::TYPE_TXT,
                        mdns::RecordData::Txt(
                            [
                                ("ty".to_string(), "Acme LaserFast 400".to_string()),
                                ("usb_mfg".to_string(), "Acme".to_string()),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    ),
                ),
                (
                    from,
                    record(
                        "studio-printer.local",
                        mdns::TYPE_A,
                        mdns::RecordData::A("192.0.2.40".parse().unwrap()),
                    ),
                ),
                (
                    from,
                    record(
                        "studio-printer.local",
                        mdns::TYPE_AAAA,
                        mdns::RecordData::Aaaa("2001:db8::28".parse().unwrap()),
                    ),
                ),
            ],
        }
    }

    #[test]
    fn mdns_evidence_lands_on_the_address_the_a_record_names() {
        let mut devices = HashMap::new();
        merge_mdns(&printer_harvest("192.0.2.40"), &policy(), &mut devices);

        let device = devices.get(&"192.0.2.40".parse().unwrap()).unwrap();
        assert!(device.services().iter().any(|s| s == "_ipp._tcp"));
        assert_eq!(
            device.best(EvidenceKind::DisplayName).unwrap().value,
            "Studio Printer"
        );
        assert_eq!(
            device.best(EvidenceKind::Model).unwrap().value,
            "Acme LaserFast 400"
        );
        assert_eq!(
            device.best(EvidenceKind::Hostname).unwrap().value,
            "studio-printer"
        );
        assert!(device.ipv6.contains(&"2001:db8::28".parse().unwrap()));
    }

    #[test]
    fn an_a_record_pointing_outside_the_scanned_network_is_not_followed() {
        // A responder claiming its service lives at 10.0.0.5 must not create a
        // device there, nor attach the evidence to an address we did not scan.
        let mut harvest = printer_harvest("192.0.2.40");
        harvest.records.retain(|(_, r)| r.rtype != mdns::TYPE_A);
        harvest.records.push((
            "192.0.2.40".parse().unwrap(),
            record(
                "studio-printer.local",
                mdns::TYPE_A,
                mdns::RecordData::A("10.0.0.5".parse().unwrap()),
            ),
        ));
        let mut devices = HashMap::new();
        merge_mdns(&harvest, &policy(), &mut devices);

        assert!(!devices.contains_key(&"10.0.0.5".parse().unwrap()));
        // It falls back to the address the packet actually came from.
        assert!(devices.contains_key(&"192.0.2.40".parse().unwrap()));
    }

    #[test]
    fn a_name_with_no_address_anywhere_creates_no_device() {
        let from: Ipv4Addr = "10.0.0.9".parse().unwrap(); // outside the policy
        let harvest = MdnsHarvest {
            packets: 1,
            records: vec![(
                from,
                record(
                    "_ipp._tcp.local",
                    mdns::TYPE_PTR,
                    mdns::RecordData::Ptr("Ghost._ipp._tcp.local".into()),
                ),
            )],
        };
        let mut devices = HashMap::new();
        merge_mdns(&harvest, &policy(), &mut devices);
        assert!(devices.is_empty());
    }

    #[test]
    fn an_mdns_instance_name_is_recorded_as_evidence_never_as_an_identity() {
        let mut devices = HashMap::new();
        merge_mdns(&printer_harvest("192.0.2.40"), &policy(), &mut devices);
        let device = devices.get(&"192.0.2.40".parse().unwrap()).unwrap();
        let identifier = device
            .of_kind(EvidenceKind::ProtocolIdentifier)
            .find(|e| e.key == "mdns_instance")
            .unwrap();
        assert!(identifier.value.contains("Studio Printer"));
        // The device is keyed by address; the instance name is a fact about it.
        assert_eq!(device.ipv4, Some("192.0.2.40".parse().unwrap()));
    }

    #[test]
    fn merging_the_same_harvest_twice_does_not_duplicate_evidence() {
        let harvest = printer_harvest("192.0.2.40");
        let mut once = HashMap::new();
        merge_mdns(&harvest, &policy(), &mut once);
        let mut twice = once.clone();
        merge_mdns(&harvest, &policy(), &mut twice);

        let a = once.get(&"192.0.2.40".parse().unwrap()).unwrap();
        let b = twice.get(&"192.0.2.40".parse().unwrap()).unwrap();
        assert_eq!(a.evidence.len(), b.evidence.len());
    }

    fn ssdp_response(body: &str) -> ssdp::Response {
        ssdp::parse(body.as_bytes()).unwrap()
    }

    #[test]
    fn ssdp_evidence_lands_on_the_responding_address() {
        let harvest = SsdpHarvest {
            responses: vec![(
                "192.0.2.1".parse().unwrap(),
                ssdp_response(
                    "HTTP/1.1 200 OK\r\n\
                     LOCATION: http://192.0.2.1:8080/desc.xml\r\n\
                     SERVER: Linux/4.4 UPnP/1.0 Acme/2.1\r\n\
                     ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\
                     USN: uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee::upnp:rootdevice\r\n",
                ),
            )],
        };
        let mut devices = HashMap::new();
        let fetches = merge_ssdp(&harvest, &mut devices);

        let device = devices.get(&"192.0.2.1".parse().unwrap()).unwrap();
        assert_eq!(
            device.best(EvidenceKind::DeviceType).unwrap().value,
            "InternetGatewayDevice"
        );
        assert_eq!(
            fetches,
            vec![(
                "192.0.2.1".parse().unwrap(),
                "http://192.0.2.1:8080/desc.xml".to_string()
            )]
        );
    }

    #[test]
    fn the_same_location_is_queued_once_however_many_times_it_is_advertised() {
        let response = ssdp_response(
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.0.2.1/d.xml\r\nST: upnp:rootdevice\r\n",
        );
        let harvest = SsdpHarvest {
            responses: vec![
                ("192.0.2.1".parse().unwrap(), response.clone()),
                ("192.0.2.1".parse().unwrap(), response.clone()),
                ("192.0.2.1".parse().unwrap(), response),
            ],
        };
        let mut devices = HashMap::new();
        assert_eq!(merge_ssdp(&harvest, &mut devices).len(), 1);
    }

    #[test]
    fn the_description_fetch_queue_is_capped() {
        let responses: Vec<(Ipv4Addr, ssdp::Response)> = (0..(MAX_DESCRIPTION_FETCHES + 30))
            .map(|i| {
                let ip: Ipv4Addr = Ipv4Addr::new(192, 0, 2, (i % 250) as u8);
                (
                    ip,
                    ssdp_response(&format!(
                        "HTTP/1.1 200 OK\r\nLOCATION: http://192.0.2.9/d{i}.xml\r\n"
                    )),
                )
            })
            .collect();
        let mut devices = HashMap::new();
        let fetches = merge_ssdp(&SsdpHarvest { responses }, &mut devices);
        assert_eq!(fetches.len(), MAX_DESCRIPTION_FETCHES);
    }

    #[test]
    fn a_description_document_enriches_the_device_it_came_from() {
        let mut device = DiscoveredDevice::new("192.0.2.1".parse().unwrap());
        let description = xml::parse(
            r#"<root><device>
                 <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
                 <friendlyName>Acme Hub 6</friendlyName>
                 <manufacturer>Acme Networks</manufacturer>
                 <modelName>Hub 6</modelName>
                 <modelNumber>AH6-2000</modelNumber>
                 <presentationURL>http://192.0.2.1/</presentationURL>
                 <serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType></service></serviceList>
               </device></root>"#,
        )
        .unwrap();
        apply_description(&mut device, &description);
        device.sort();

        assert_eq!(
            device.best(EvidenceKind::DisplayName).unwrap().value,
            "Acme Hub 6"
        );
        assert_eq!(
            device.best(EvidenceKind::Manufacturer).unwrap().value,
            "Acme Networks"
        );
        assert_eq!(device.best(EvidenceKind::Model).unwrap().value, "Hub 6");
        assert!(device.services().iter().any(|s| s == "WANIPConnection"));
        assert_eq!(
            device.best(EvidenceKind::Url).unwrap().value,
            "http://192.0.2.1/"
        );
    }

    #[test]
    fn a_generic_friendly_name_is_recorded_at_low_confidence() {
        let mut device = DiscoveredDevice::new("192.0.2.1".parse().unwrap());
        let description =
            xml::parse("<root><device><friendlyName>UPnP Device</friendlyName></device></root>")
                .unwrap();
        apply_description(&mut device, &description);
        assert_eq!(
            device.best(EvidenceKind::DisplayName).unwrap().confidence,
            Confidence::Low
        );
    }

    #[test]
    fn a_full_pass_over_fixtures_produces_a_named_classified_printer() {
        // The whole pipeline without a socket: real mDNS records, the real
        // merger, the real classifier and the real name resolver.
        let mut devices = HashMap::new();
        merge_mdns(&printer_harvest("192.0.2.40"), &policy(), &mut devices);
        let device = devices.get_mut(&"192.0.2.40".parse().unwrap()).unwrap();
        device.sort();

        let classification = classify(
            Some(device),
            &ClassifyFacts {
                open_ports: &[631, 9100],
                vendor: Some("Acme Corp"),
                ..Default::default()
            },
        );
        assert_eq!(classification.device_type, DeviceType::Printer);
        assert_eq!(classification.confidence, Confidence::High);

        let resolved = names::resolve(
            &names::NameInputs {
                ip: Some("192.0.2.40"),
                ..Default::default()
            },
            Some(device),
        );
        assert_eq!(resolved.name, "Studio Printer");
        assert_eq!(resolved.source, DiscoverySource::Mdns);
    }
}
