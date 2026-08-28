//! SQLite persistence for scan history and the device inventory (bundled
//! rusqlite — no system SQLite).
//!
//! # Schema shape
//!
//! v1.6 stored two tables: `scans` and `hosts`, one `hosts` row per device per
//! scan. That is already the right shape for an observation log, so v1.7 keeps
//! both tables and their data exactly where they are, and adds:
//!
//! * `devices` — one row per physical device, matched by the identity rules in
//!   [`crate::inventory`], carrying the operator's name, status and notes.
//! * `hosts.device_id` — links every observation, old and new, to its device.
//! * Change counts and comparison metadata on `scans`, computed once at save
//!   time so the history list never runs a query per row.
//!
//! Migrating in place rather than moving data into a new observations table
//! means an existing v1.6.4 database keeps every scan it ever recorded, and the
//! upgrade is a handful of `ALTER TABLE`s plus one backfill pass.
//!
//! # Persistent inventory and change events (v1.8.0)
//!
//! v1.8 adds no new observation storage. The inventory view is a *query* over
//! the tables above ([`Db::inventory`]), and the only new table is
//! `change_events`: one normalized row per device, per scan, per kind of change,
//! written once when a completed scan is saved and carrying its own review
//! state. Deriving the inventory rather than materialising it means a rename, a
//! status change or a deleted scan can never leave two copies of the truth.
//!
//! # Network scopes (v1.7.1)
//!
//! Device identity is scoped to a *network scope*: one row per physical
//! network, resolved from the canonical target plus — for local scans — the
//! default gateway's MAC address, which is what tells two unrelated
//! `192.168.1.0/24` networks apart. Every scan and every device belongs to a
//! scope, matching never crosses scope boundaries, and neither do names,
//! notes, status or comparison baselines.
//!
//! # Comparison rules (v1.7.1)
//!
//! A scan may only be compared with an earlier scan that (a) completed — a
//! cancelled scan did not observe its whole target, so absence from it proves
//! nothing — (b) belongs to the same network scope, (c) covers the same
//! canonical target, and (d) carries the same coverage key (ports and
//! discovery mode, see [`crate::signature`]).
//!
//! # Migrations
//!
//! Every migration is idempotent: `schema_meta` records the version reached,
//! `ALTER TABLE ... ADD COLUMN` failures for already-present columns are
//! ignored (SQLite has no `ADD COLUMN IF NOT EXISTS`), backfills only touch
//! rows that still need them, and the v3 devices-table rebuild runs inside a
//! transaction and only when the old shape is detected. Opening a database
//! repeatedly, or opening an already-current database, changes nothing.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::inventory::{
    self, ChangeKind, ChangeState, ChangeType, Device, DeviceStatus, IdentifiedHost,
    IdentitySource, PresenceState, ScanComparison,
};
use crate::ipparse;
use crate::scanner::{HostResult, ScanResult};
use crate::signature;

/// Current schema version. Bump when a migration is added below.
const SCHEMA_VERSION: i64 = 6;

/// Which generation of the naming rules wrote a device's stored detected name.
///
/// v1.8.3 tidies names that v1.8.2 kept as they arrived: a UDN masquerading as
/// a friendly name, a service-instance suffix, a manufacturer the model already
/// contains. Every one of those is an improvement, and every one of them would
/// have looked like the device renaming itself on the first scan after the
/// upgrade — one "detected name changed" event per device, all at once, for
/// nothing that happened on the network.
///
/// So the generation is recorded beside the name. When a device's stored record
/// predates the current rules, the first comparison against it is silent for
/// the name and the model; everything else about that scan is compared
/// normally, and every scan afterwards compares the name normally too, because
/// by then both sides were written by the same rules.
pub const NAMING_RULES_VERSION: i64 = 2;

/// How many consecutive full-discovery scans must miss an advertised service
/// before ArcScan believes it has gone.
///
/// One is far too few. Multicast is lossy by design — a single dropped response
/// is ordinary on Wi-Fi — and reporting a removal on one miss would fill the
/// inbox with services that were never gone. Two consecutive misses, each from a
/// scan that ran discovery to completion, is the conservative rule.
pub const SERVICE_ABSENCE_MISSES: i64 = 2;

/// Most previous addresses kept per device in an inventory row. The full trail
/// is in the device drawer; the table and the export only need the recent ones,
/// and an unbounded list would make one long-lived device dominate an export.
const PREVIOUS_IP_LIMIT: usize = 8;

/// Largest change-event page the inbox will load in one call. Beyond this the
/// list is truncated (newest first) and the UI says so, so a database with
/// years of history cannot stall the view.
pub const CHANGE_EVENT_LIMIT: i64 = 5_000;

/// Why a cancelled scan carries no comparison.
pub const PARTIAL_SCAN_REASON: &str = "This scan was stopped before every address was checked, \
     so missing devices and complete network changes cannot be determined reliably.";

/// Why two scans of the same target were not compared.
pub const COVERAGE_MISMATCH_REASON: &str = "These scans checked different ports or used \
     different discovery modes, so ArcScan did not compare them.";

/// Why a scan recorded before coverage keys existed cannot be compared.
pub const LEGACY_COVERAGE_REASON: &str = "This scan was recorded by an earlier version of \
     ArcScan that did not save which ports it checked, so it cannot be compared safely.";

/// How many observations a device detail view loads. Deep history is available
/// through the scan list; the drawer only needs the recent trail, so a device
/// seen by thousands of scans still opens instantly.
const DEVICE_HISTORY_LIMIT: i64 = 50;

pub struct Db {
    conn: Mutex<Option<Connection>>,
}

struct DbConnectionGuard<'a> {
    guard: MutexGuard<'a, Option<Connection>>,
}

impl Deref for DbConnectionGuard<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("DbConnectionGuard is only constructed for an open database")
    }
}

impl DerefMut for DbConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("DbConnectionGuard is only constructed for an open database")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub id: i64,
    pub target: String,
    /// Normalized target, used to decide which scans may be compared.
    pub target_key: String,
    pub profile: Option<String>,
    pub created_at: String,
    pub duration_ms: i64,
    /// Addresses the target expands to.
    pub scanned: i64,
    /// Addresses actually probed. Lower than `scanned` for a cancelled scan.
    pub probed: i64,
    pub host_count: i64,
    pub new_count: i64,
    pub missing_count: i64,
    pub changed_count: i64,
    /// `completed` or `cancelled`.
    pub status: String,
    pub baseline_scan_id: Option<i64>,
    /// The network scope this scan belongs to.
    #[serde(default)]
    pub network_scope_id: Option<i64>,
    /// The scope's display name, joined in so history needs no second query.
    #[serde(default)]
    pub scope_name: Option<String>,
    /// Ports-and-discovery-mode signature; see [`crate::signature`].
    #[serde(default)]
    pub coverage_key: String,
    /// What the scan's local-discovery pass managed: `full`, `partial` or
    /// `none`. Shown in History, and the gate for discovery-derived
    /// comparisons. Deliberately separate from `coverage_key`, so two scans
    /// still compare on hosts and ports whatever their discovery differed by.
    #[serde(default)]
    pub discovery_mode: String,
    /// Counts and a skip reason, as recorded by the scan. Opaque JSON to the
    /// database; the interface renders it.
    #[serde(default)]
    pub discovery_summary: Option<String>,
    /// How well the discovery pass went, in one of four words: `complete`,
    /// `limited`, `skipped` or `interrupted`.
    ///
    /// Derived here rather than in the interface so the rule lives with the
    /// report that defines it, and so History does not parse a JSON blob per
    /// row to draw one line. Distinct from `discovery_mode`, which gates
    /// comparison and whose meaning may not move; this one is for a person.
    #[serde(default)]
    pub discovery_quality: String,
    /// Why the pass was less than complete, in one short phrase, or `None`.
    ///
    /// Only ever something ArcScan observed — a socket that would not open, a
    /// cap that was reached, a description that was refused. Never a diagnosis:
    /// ArcScan cannot see a firewall and so never blames one.
    #[serde(default)]
    pub discovery_quality_reason: Option<String>,
}

/// One persistent network scope: a physical network as ArcScan understands it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkScope {
    pub id: i64,
    pub stable_key: String,
    pub display_name: String,
    pub canonical_target: Option<String>,
    pub gateway_mac: Option<String>,
    pub interface_hint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub device_count: i64,
    pub scan_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDetail {
    #[serde(flatten)]
    pub summary: ScanSummary,
    pub hosts: Vec<HostResult>,
    /// Device id and operator name for each host, positionally aligned with
    /// `hosts`, so the UI can show friendly names without a query per row.
    pub devices: Vec<HostDevice>,
}

/// The inventory identity attached to one observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostDevice {
    pub ip: String,
    pub device_id: Option<i64>,
    pub custom_name: Option<String>,
    pub status: DeviceStatus,
    pub first_seen: Option<String>,
}

/// What `save_scan` returns: the new scan's id and how it differs from the most
/// recent compatible scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedScan {
    pub scan_id: i64,
    pub comparison: ScanComparison,
}

/// One historical sighting of a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceObservation {
    pub scan_id: i64,
    pub scan_target: String,
    pub observed_at: String,
    pub ip: String,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<u16>,
    pub response_ms: Option<u64>,
    pub icmp_ms: Option<f64>,
    pub tcp_ms: Option<f64>,
    pub ttl: Option<u8>,
    pub os_guess: Option<String>,
}

/// One row of the persistent Inventory.
///
/// Everything here comes from the two queries in [`Db::inventory`]: no per-row
/// lookups, no full observation history, and no note bodies — only whether a
/// note exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryRow {
    pub device_id: i64,
    pub network_scope_id: Option<i64>,
    /// The network's friendly name, or its automatic label.
    pub network_name: Option<String>,
    pub identity_source: IdentitySource,
    /// Friendly name, hostname, manufacturer or address, in that order.
    pub display_name: String,
    pub custom_name: Option<String>,
    pub hostname: Option<String>,
    /// Address in the most recent observation.
    pub current_ip: Option<String>,
    /// Earlier addresses, newest first, without the current one.
    pub previous_ips: Vec<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub os_guess: Option<String>,
    /// How the operator classified the device.
    pub status: DeviceStatus,
    /// What the latest completed scan says about the device.
    pub presence: PresenceState,
    pub first_seen: String,
    pub last_seen: String,
    /// The scan the presence state was decided from, when there is one.
    pub last_completed_scan_id: Option<i64>,
    pub last_completed_scan_at: Option<String>,
    pub observation_count: i64,
    /// Open ports in the most recent observation.
    pub open_ports: Vec<u16>,
    /// True when the device carries notes.
    pub notes_present: bool,
    /// The opening of the note, so search can reach it. Deliberately not the
    /// whole body: the table shows an indicator, and loading thousands of full
    /// notes to render a dot would be wasted work.
    pub notes_excerpt: Option<String>,
    pub latest_response_ms: Option<i64>,
    pub latest_icmp_ms: Option<f64>,
    pub latest_tcp_ms: Option<f64>,
    /// What local discovery established about this device, if anything. Absent
    /// for every device no discovery-capable scan has reached — which is every
    /// device on an install that has just upgraded.
    #[serde(default)]
    pub discovery: Option<InventoryDiscovery>,
    /// The operator's device-type correction, or `None` for Auto.
    ///
    /// Carried on the row rather than inside `discovery`, because a device
    /// discovery has never reached still has a type the operator may have set,
    /// and burying it inside an absent record would make it unreachable.
    #[serde(default)]
    pub user_device_type: Option<String>,
}

/// The discovery facts the Inventory table, its search and its export use.
///
/// A narrower set than the drawer shows: this one is loaded for every row, so
/// it carries what a table column or a search term can reach and nothing more.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryDiscovery {
    pub detected_name: Option<String>,
    /// What ArcScan detected. The type shown on screen is
    /// [`InventoryRow::user_device_type`] when there is one, and this
    /// otherwise; the interface settles that in one place rather than in each
    /// component.
    pub device_type: String,
    /// The detected confidence, already reduced where the evidence behind it
    /// has gone stale. See [`InventoryDiscovery::evidence_freshness`].
    pub type_confidence: String,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
    pub services: Vec<String>,
    pub sources: Vec<String>,
    pub last_discovered_at: Option<String>,
    /// How current the freshest piece of mDNS or SSDP evidence behind this
    /// record is: `current`, `aging` or `stale`. `current` when there is no
    /// evidence at all, because nothing has gone stale.
    #[serde(default)]
    pub evidence_freshness: String,
}

/// A network as the Inventory and Changes filters offer it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkOption {
    pub id: i64,
    pub name: String,
    pub device_count: i64,
}

/// The whole Inventory plus the counts its header shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventorySummary {
    pub rows: Vec<InventoryRow>,
    /// Networks that actually hold devices, for the network filter.
    pub networks: Vec<NetworkOption>,
    pub present: i64,
    pub missing: i64,
    pub unknown: i64,
    /// True when no completed scan anywhere can decide presence, so the UI can
    /// explain why every device reads Unknown.
    pub needs_completed_scan: bool,
}

/// One persisted change, as the Changes inbox shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub id: i64,
    /// Deterministic identity: the same change recorded twice is one row.
    pub event_key: String,
    /// The scan that found the change. Null once that scan has been pruned.
    pub scan_id: Option<i64>,
    pub baseline_scan_id: Option<i64>,
    pub network_scope_id: Option<i64>,
    pub network_name: Option<String>,
    pub device_id: Option<i64>,
    /// The device's current name, falling back to the label recorded with the
    /// event when the device is gone.
    pub device_label: String,
    pub ip: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub change_type: ChangeType,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    /// Ports that opened, for `ports_changed`. Structured, not display text.
    pub opened_ports: Vec<u16>,
    /// Ports that closed, for `ports_changed`.
    pub closed_ports: Vec<u16>,
    pub state: ChangeState,
    /// When the change was recorded.
    pub created_at: String,
    /// When the scan that found it ran, kept so the record survives pruning.
    pub scan_at: Option<String>,
    pub baseline_at: Option<String>,
    pub acknowledged_at: Option<String>,
    /// The device's current classification, so Trust and Ignore can be offered
    /// only where they would do something.
    pub device_status: Option<DeviceStatus>,
}

/// A page of change events plus the counts the inbox header shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeFeed {
    pub events: Vec<ChangeEvent>,
    pub unreviewed: i64,
    pub total: i64,
    /// True when older events exist beyond the page that was returned.
    pub truncated: bool,
    /// The newest scan that existed when this database was upgraded to the
    /// v1.8 schema. Changes are recorded for scans after it, so an inbox that
    /// is empty on an upgraded install can say why instead of looking broken.
    /// Zero on a database that has never held a scan.
    pub starts_after_scan_id: i64,
}

/// What a bulk action actually managed to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkOutcome {
    pub updated: usize,
    /// Ids that no longer exist. The transaction still commits the rest, and
    /// the UI reports the shortfall rather than pretending it succeeded.
    pub missing: Vec<i64>,
}

/// Everything the device drawer needs, in one round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDetail {
    pub device: Device,
    /// Most recent observations, newest first.
    pub observations: Vec<DeviceObservation>,
    /// Addresses this device has held, newest first.
    pub previous_ips: Vec<String>,
    /// Field changes between the two most recent observations.
    pub recent_changes: Vec<inventory::FieldChange>,
    /// Persisted change events for this device, newest first. Distinct from
    /// `recent_changes`: these are the reviewable records the Changes inbox
    /// works with, not a diff computed on the spot.
    #[serde(default)]
    pub events: Vec<ChangeEvent>,
    /// The network this device belongs to, named.
    #[serde(default)]
    pub network_name: Option<String>,
    /// What the latest completed scan says about the device.
    #[serde(default)]
    pub presence: PresenceState,
    /// Everything local discovery has established about this device, with the
    /// evidence behind it. Absent until a discovery-capable scan reaches it.
    #[serde(default)]
    pub discovery: Option<DeviceDiscovery>,
}

/// The full discovery record for one device, as the drawer shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDiscovery {
    pub detected_name: Option<String>,
    pub name_source: Option<String>,
    pub device_type: String,
    pub type_confidence: String,
    /// Plain-language facts behind the type.
    pub type_evidence: Vec<String>,
    /// Other types the evidence supported, as `Type · confidence`.
    pub type_conflicts: Vec<String>,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub serial_number: Option<String>,
    pub mdns_hostname: Option<String>,
    pub ssdp_friendly_name: Option<String>,
    pub services: Vec<String>,
    pub sources: Vec<String>,
    pub alternate_names: Vec<String>,
    /// Learned from mDNS, shown as supplemental information. ArcScan scans IPv4.
    pub ipv6_addresses: Vec<String>,
    pub presentation_url: Option<String>,
    pub first_discovered_at: Option<String>,
    pub last_discovered_at: Option<String>,
    /// The durable evidence rows, newest-seen first. This is the persistent
    /// record, kept clearly apart from the per-scan observation history the
    /// drawer lists separately.
    pub evidence: Vec<DiscoveryEvidenceRow>,
    /// How current the freshest mDNS or SSDP claim behind this record is.
    #[serde(default)]
    pub evidence_freshness: String,
    /// The confidence the classifier reached, before any reduction for stale
    /// evidence. Kept so the drawer can explain a reduction rather than only
    /// show its result.
    #[serde(default)]
    pub raw_type_confidence: String,
}

/// One stored claim about a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryEvidenceRow {
    pub source: String,
    pub kind: String,
    pub key: String,
    pub value: String,
    pub confidence: String,
    pub first_seen: String,
    pub last_seen: String,
    /// `current`, `aging` or `stale`.
    #[serde(default)]
    pub freshness: String,
    /// Consecutive qualifying discovery scans that did not re-observe this
    /// claim. Shown as "last seen N discovery scans ago" — a count of scans,
    /// never a number of days, because ArcScan only learns when it runs.
    #[serde(default)]
    pub misses: i64,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Mutex::new(Some(conn)),
        })
    }

    /// Open an in-memory database. Used by the migration and inventory tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let mut conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Mutex::new(Some(conn)),
        })
    }

    fn lock(&self) -> Result<DbConnectionGuard<'_>, String> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| "The scan database is unavailable because a previous write failed.")?;
        if guard.is_none() {
            return Err("The scan database is closed because ArcScan is shutting down.".into());
        }
        Ok(DbConnectionGuard { guard })
    }

    /// Stop new database work, checkpoint WAL state and close SQLite.
    ///
    /// Portable calls this on Tauri's Exit event before the WebView is torn
    /// down. Taking the option waits for any in-flight command holding the
    /// mutex, then makes every later command fail instead of reopening or
    /// writing while the owned session is being removed.
    #[cfg(any(feature = "portable", test))]
    pub fn shutdown(&self) -> Result<(), String> {
        let mut slot = self
            .conn
            .lock()
            .map_err(|_| "The scan database could not be closed cleanly.")?;
        let Some(connection) = slot.take() else {
            return Ok(());
        };

        if let Err(error) = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            *slot = Some(connection);
            return Err(format!("Could not checkpoint the scan database: {error}"));
        }
        match connection.close() {
            Ok(()) => Ok(()),
            Err((connection, error)) => {
                *slot = Some(connection);
                Err(format!("Could not close the scan database: {error}"))
            }
        }
    }

    /// Persist a scan, fold its hosts into the device inventory, and compare it
    /// with the most recent compatible scan. One transaction, so an interrupted
    /// save never leaves a half-recorded scan.
    ///
    /// A cancelled scan is saved in full — target, coverage, timing, the hosts
    /// found before Stop — but is never compared: it did not observe its whole
    /// target, so devices absent from it cannot be called missing and ports not
    /// probed cannot be called closed. It also never becomes a baseline, which
    /// [`find_baseline`] enforces with `status = 'completed'`.
    pub fn save_scan(&self, result: &ScanResult) -> Result<SavedScan, String> {
        let mut conn = self.lock()?;
        let created_at = chrono::Local::now().to_rfc3339();
        // A target that reached the scanner always parses, so a failure here can
        // only mean a hand-crafted call; fall back to the raw string rather than
        // refusing to save real results.
        let target_key =
            ipparse::canonical_key(&result.target).unwrap_or_else(|_| result.target.clone());
        let coverage_key = signature::coverage_key(&result.ports, result.arp_assist);
        let execution_settings = result
            .execution
            .as_ref()
            .and_then(|e| serde_json::to_string(e).ok());
        let tx = conn.transaction().map_err(sql_err)?;

        let scope_id = resolve_scope(&tx, result, &target_key, &created_at)?;

        // Pick the baseline before inserting, so the new scan cannot be its own
        // comparison point. A cancelled scan gets none at all.
        let baseline = if result.cancelled {
            None
        } else {
            find_baseline(&tx, Some(scope_id), &target_key, &coverage_key, None)?
        };
        let baseline_hosts = match &baseline {
            Some(b) => load_identified(&tx, b.id)?,
            None => Vec::new(),
        };

        let status = if result.cancelled {
            "cancelled"
        } else {
            "completed"
        };
        // A cancelled scan can never claim a full discovery pass, whatever the
        // report says, because the phases after Stop did not run.
        let discovery_report = result.discovery.clone().unwrap_or_default();
        let discovery_mode = if result.cancelled {
            crate::discovery::model::DiscoveryMode::None
        } else {
            discovery_report.mode()
        };
        let discovery_summary = serde_json::to_string(&discovery_report).ok();

        tx.execute(
            "INSERT INTO scans
                (target, target_key, profile, created_at, duration_ms, scanned, probed, status,
                 network_scope_id, coverage_key, execution_settings, discovery_mode,
                 discovery_summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                result.target,
                target_key,
                result.profile,
                created_at,
                result.duration_ms as i64,
                result.scanned as i64,
                result.probed as i64,
                status,
                scope_id,
                coverage_key,
                execution_settings,
                discovery_mode.as_str(),
                discovery_summary,
            ],
        )
        .map_err(sql_err)?;
        let scan_id = tx.last_insert_rowid();

        // Hosts found before a Stop are genuine observations, so they fold into
        // the inventory either way.
        let pass = DiscoveryPass {
            full: discovery_mode == crate::discovery::model::DiscoveryMode::Full,
            descriptions: discovery_report.descriptions_fetched > 0,
        };
        let mut current: Vec<IdentifiedHost> = Vec::with_capacity(result.hosts.len());
        // Device id -> (before, after) discovery, for the change pass below.
        let mut discovery_changes: HashMap<i64, (Option<DiscoverySnapshot>, DiscoverySnapshot)> =
            HashMap::new();
        for host in &result.hosts {
            let record = upsert_device(&tx, scope_id, host, &created_at)?;
            insert_observation(&tx, scan_id, record.id, host)?;
            if let Some(discovery) = &host.discovery {
                let before = read_discovery_snapshot(&tx, record.id)?;
                let after = write_discovery(
                    &tx,
                    record.id,
                    scope_id,
                    scan_id,
                    discovery,
                    &created_at,
                    pass,
                )?;
                discovery_changes.insert(record.id, (before, after));
            }
            let mut identified = IdentifiedHost::from_host(host.clone());
            identified.device_id = Some(record.id);
            identified.identity_key = record.identity_key;
            identified.custom_name = record.custom_name;
            identified.previously_known = record.existed;
            current.push(identified);
        }

        // With no baseline there is nothing for a device to be new *against*, so
        // the comparison is empty rather than listing the whole network as new
        // arrivals the first time a target is scanned.
        let comparison = if result.cancelled {
            ScanComparison::empty(scan_id, PARTIAL_SCAN_REASON)
        } else {
            match &baseline {
                None => ScanComparison::empty(
                    scan_id,
                    "This is the first completed scan with this target and coverage, so there \
                     is nothing to compare it with yet.",
                ),
                Some(b) => {
                    let mut c = inventory::compare(scan_id, &baseline_hosts, &current);
                    c.baseline_scan_id = Some(b.id);
                    c.baseline_created_at = Some(b.created_at.clone());
                    c.baseline_target = Some(b.target.clone());
                    c
                }
            }
        };

        let new_count = comparison
            .added
            .iter()
            .filter(|d| d.kind == ChangeKind::New)
            .count() as i64;
        tx.execute(
            "UPDATE scans SET new_count = ?1, missing_count = ?2, changed_count = ?3,
                              baseline_scan_id = ?4
             WHERE id = ?5",
            params![
                new_count,
                comparison.removed.len() as i64,
                comparison.changed.len() as i64,
                baseline.as_ref().map(|b| b.id),
                scan_id,
            ],
        )
        .map_err(sql_err)?;

        // Persist the changes for the inbox. A cancelled scan has no baseline
        // and produces no comparison, so this is naturally a no-op for it: a
        // scan that did not check every address must never create an actionable
        // change event.
        if let Some(b) = &baseline {
            record_change_events(&tx, scan_id, scope_id, b, &comparison, &created_at)?;

            // Discovery-derived events need equivalent capability on both
            // sides. Comparing a scan that listened against one that could not
            // would report every advertisement as newly appeared, which is the
            // definition of a noisy inbox.
            if pass.full && b.discovery_mode == "full" {
                let ignored: std::collections::HashSet<i64> = {
                    let mut stmt = tx
                        .prepare(
                            "SELECT id FROM devices
                             WHERE network_scope_id = ?1 AND status = 'ignored'",
                        )
                        .map_err(sql_err)?;
                    let rows = stmt
                        .query_map(params![scope_id], |r| r.get::<_, i64>(0))
                        .map_err(sql_err)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(sql_err)?;
                    rows.into_iter().collect()
                };
                // Sorted so a scan writes its events in a deterministic order.
                let mut device_ids: Vec<i64> = discovery_changes.keys().copied().collect();
                device_ids.sort_unstable();
                for device_id in device_ids {
                    let Some((before, after)) = discovery_changes.get(&device_id) else {
                        continue;
                    };
                    let entry = current.iter().find(|h| h.device_id == Some(device_id));
                    let (label, ip) = match entry {
                        Some(h) => (
                            inventory::display_name(
                                h.custom_name.as_deref(),
                                h.host.hostname.as_deref(),
                                h.host.vendor.as_deref(),
                                &h.host.ip,
                            ),
                            h.host.ip.clone(),
                        ),
                        None => continue,
                    };
                    record_discovery_events(
                        &tx,
                        &DiscoverySubject {
                            scan_id,
                            scope_id,
                            baseline: b,
                            device_id,
                            label: &label,
                            ip: &ip,
                            now: &created_at,
                            ignored: ignored.contains(&device_id),
                        },
                        before.as_ref(),
                        after,
                    )?;
                }
            }
        }

        tx.commit().map_err(sql_err)?;
        Ok(SavedScan {
            scan_id,
            comparison,
        })
    }

    pub fn list_scans(&self) -> Result<Vec<ScanSummary>, String> {
        let conn = self.lock()?;
        // host_count comes from a grouped join rather than a correlated subquery
        // per row, so a long history lists in one pass.
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.target, s.target_key, s.profile, s.created_at, s.duration_ms,
                        s.scanned, s.probed, COUNT(h.id), s.new_count, s.missing_count,
                        s.changed_count, s.status, s.baseline_scan_id,
                        s.network_scope_id, ns.display_name, s.coverage_key,
                        s.discovery_mode, s.discovery_summary
                 FROM scans s
                 LEFT JOIN hosts h ON h.scan_id = s.id
                 LEFT JOIN network_scopes ns ON ns.id = s.network_scope_id
                 GROUP BY s.id
                 ORDER BY s.id DESC",
            )
            .map_err(sql_err)?;
        let rows = stmt.query_map([], read_summary).map_err(sql_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_err)
    }

    /// Every known network scope, with how much history it anchors.
    pub fn list_network_scopes(&self) -> Result<Vec<NetworkScope>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT ns.id, ns.stable_key, ns.display_name, ns.canonical_target,
                        ns.gateway_mac, ns.interface_hint, ns.created_at, ns.updated_at,
                        (SELECT COUNT(*) FROM devices d WHERE d.network_scope_id = ns.id),
                        (SELECT COUNT(*) FROM scans s WHERE s.network_scope_id = ns.id)
                 FROM network_scopes ns
                 ORDER BY ns.updated_at DESC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(NetworkScope {
                    id: row.get(0)?,
                    stable_key: row.get(1)?,
                    display_name: row.get(2)?,
                    canonical_target: row.get(3)?,
                    gateway_mac: row.get(4)?,
                    interface_hint: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    device_count: row.get(8)?,
                    scan_count: row.get(9)?,
                })
            })
            .map_err(sql_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_err)
    }

    /// Give a scope an operator-chosen name, e.g. `Office LAN` or `Client VPN`.
    pub fn rename_network_scope(&self, id: i64, name: String) -> Result<(), String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("A network name cannot be empty.".into());
        }
        if name.chars().count() > 80 {
            return Err("Network names are limited to 80 characters.".into());
        }
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE network_scopes SET display_name = ?1 WHERE id = ?2",
                params![name, id],
            )
            .map_err(sql_err)?;
        if updated == 0 {
            return Err(format!("Network {id} no longer exists."));
        }
        Ok(())
    }

    pub fn get_scan(&self, id: i64) -> Result<ScanDetail, String> {
        let conn = self.lock()?;
        let summary = conn
            .query_row(
                "SELECT s.id, s.target, s.target_key, s.profile, s.created_at, s.duration_ms,
                        s.scanned, s.probed, COUNT(h.id), s.new_count, s.missing_count,
                        s.changed_count, s.status, s.baseline_scan_id,
                        s.network_scope_id, ns.display_name, s.coverage_key,
                        s.discovery_mode, s.discovery_summary
                 FROM scans s
                 LEFT JOIN hosts h ON h.scan_id = s.id
                 LEFT JOIN network_scopes ns ON ns.id = s.network_scope_id
                 WHERE s.id = ?1
                 GROUP BY s.id",
                params![id],
                read_summary,
            )
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| format!("Scan {id} is no longer in the history."))?;

        // One joined query returns the observations and their device identities
        // together, so opening a saved scan costs a single statement.
        let mut stmt = conn
            .prepare(
                "SELECT h.ip, h.hostname, h.mac, h.vendor, h.open_ports, h.response_ms,
                        h.icmp_ms, h.tcp_ms, h.ttl, h.os_guess, h.last_seen,
                        h.device_id, d.custom_name, d.status, d.first_seen
                 FROM hosts h LEFT JOIN devices d ON d.id = h.device_id
                 WHERE h.scan_id = ?1
                 ORDER BY h.id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![id], |row| {
                let host = read_host(row)?;
                let device = HostDevice {
                    ip: host.ip.clone(),
                    device_id: row.get(11)?,
                    custom_name: row.get(12)?,
                    status: row
                        .get::<_, Option<String>>(13)?
                        .as_deref()
                        .map(DeviceStatus::parse)
                        .unwrap_or_default(),
                    first_seen: row.get(14)?,
                };
                Ok((host, device))
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;

        let (hosts, devices) = rows.into_iter().unzip();
        Ok(ScanDetail {
            summary,
            hosts,
            devices,
        })
    }

    /// Compare a saved scan with the most recent compatible scan that precedes
    /// it. Compatibility means the same network scope, the same normalized
    /// target, the same coverage key, and a completed baseline: a Quick LAN
    /// sweep and a Full TCP sweep of one subnet see different services, so
    /// diffing them would report invented port changes, and a cancelled scan
    /// did not see its whole target, so it can neither be compared nor serve
    /// as a baseline.
    pub fn compare_scan(&self, id: i64) -> Result<ScanComparison, String> {
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction().map_err(sql_err)?;
        let Some((target_key, coverage_key, scope_id, status)) = tx
            .query_row(
                "SELECT target_key, coverage_key, network_scope_id, status
                 FROM scans WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?
        else {
            return Err(format!("Scan {id} is no longer in the history."));
        };

        if status != "completed" {
            return Ok(ScanComparison::empty(id, PARTIAL_SCAN_REASON));
        }
        // A pre-v1.7.1 Custom or Full TCP scan never recorded its port set, so
        // its coverage is unknown and it fails safely: comparable with nothing.
        if coverage_key.starts_with("legacy:") {
            return Ok(ScanComparison::empty(id, LEGACY_COVERAGE_REASON));
        }

        let baseline = find_baseline(&tx, scope_id, &target_key, &coverage_key, Some(id))?;
        let Some(baseline) = baseline else {
            // Distinguish "never scanned before" from "scanned with different
            // coverage", so the UI can explain why nothing was compared.
            let incompatible_earlier: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM scans
                     WHERE network_scope_id IS ?1 AND target_key = ?2
                       AND status = 'completed' AND id < ?3",
                    params![scope_id, target_key, id],
                    |r| r.get(0),
                )
                .map_err(sql_err)?;
            let reason = if incompatible_earlier > 0 {
                COVERAGE_MISMATCH_REASON
            } else {
                "No earlier completed scan with this target and coverage exists, so there is \
                 nothing to compare against."
            };
            return Ok(ScanComparison::empty(id, reason));
        };

        let before = load_identified(&tx, baseline.id)?;
        let after = load_identified(&tx, id)?;
        let mut comparison = inventory::compare(id, &before, &after);
        comparison.baseline_scan_id = Some(baseline.id);
        comparison.baseline_created_at = Some(baseline.created_at);
        comparison.baseline_target = Some(baseline.target);
        Ok(comparison)
    }

    pub fn delete_scan(&self, id: i64) -> Result<(), String> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(sql_err)?;
        tx.execute("DELETE FROM scans WHERE id = ?1", params![id])
            .map_err(sql_err)?;
        clear_deleted_scan_links(&tx)?;
        tx.commit().map_err(sql_err)
    }

    /// Drop the oldest scans, keeping the newest `keep`. Devices survive so
    /// labels, notes and first-seen dates are never lost to retention, and so do
    /// change records.
    pub fn prune_history(&self, keep: i64) -> Result<usize, String> {
        if keep < 1 {
            return Err("History retention must keep at least one scan.".into());
        }
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(sql_err)?;
        let removed = tx
            .execute(
                "DELETE FROM scans WHERE id NOT IN
                    (SELECT id FROM scans ORDER BY id DESC LIMIT ?1)",
                params![keep],
            )
            .map_err(sql_err)?;
        clear_deleted_scan_links(&tx)?;
        tx.commit().map_err(sql_err)?;
        Ok(removed)
    }

    /// The whole device inventory, newest sighting first.
    pub fn list_devices(&self) -> Result<Vec<Device>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.network_scope_id, d.identity_key, d.identity_source, d.mac,
                        d.custom_name, d.hostname, d.vendor, d.last_ip, d.first_seen,
                        d.last_seen, d.status, d.notes, COUNT(h.id), d.user_device_type
                 FROM devices d LEFT JOIN hosts h ON h.device_id = d.id
                 GROUP BY d.id
                 ORDER BY d.last_seen DESC",
            )
            .map_err(sql_err)?;
        let rows = stmt.query_map([], read_device).map_err(sql_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_err)
    }

    /// The persistent Inventory: one row per device across every scan, with the
    /// presence rules in [`PresenceState`] applied.
    ///
    /// Two statements, whatever the size of the database. Everything a row needs
    /// — the latest observation, the observation count, the previous addresses,
    /// whether a note exists, and the presence verdict — is computed set-wise in
    /// SQL rather than by asking a question per device. Note bodies and full
    /// observation histories are deliberately not loaded: the drawer fetches
    /// those for the one device it is showing.
    pub fn inventory(&self) -> Result<InventorySummary, String> {
        let conn = self.lock()?;

        // Addresses first, so the main pass can attach them without a lookup.
        // Grouped by device and address, ordered newest-sighting-first.
        let mut ip_history: HashMap<i64, Vec<String>> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT device_id, ip FROM (
                         SELECT h.device_id AS device_id, h.ip AS ip, MAX(h.scan_id) AS seen
                         FROM hosts h
                         WHERE h.device_id IS NOT NULL
                         GROUP BY h.device_id, h.ip
                     )
                     ORDER BY device_id ASC, seen DESC",
                )
                .map_err(sql_err)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .map_err(sql_err)?;
            for row in rows {
                let (device_id, ip) = row.map_err(sql_err)?;
                let entry = ip_history.entry(device_id).or_default();
                // One extra, because the current address is dropped below.
                if entry.len() <= PREVIOUS_IP_LIMIT {
                    entry.push(ip);
                }
            }
        }

        let mut stmt = conn.prepare(INVENTORY_SQL).map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                let device_id: i64 = row.get(0)?;
                let custom_name: Option<String> = row.get(4)?;
                let hostname: Option<String> = row.get(5)?;
                let vendor: Option<String> = row.get(6)?;
                let last_ip: Option<String> = row.get(8)?;
                let latest_ip: Option<String> = row.get(14)?;
                let current_ip = latest_ip.or(last_ip);
                let present: bool = row.get(22)?;
                let comparable: bool = row.get(23)?;
                let detected_name: Option<String> = row.get(26)?;
                let device_type: Option<String> = row.get(27)?;
                // NULL means Auto. A value is checked against the type
                // vocabulary on the way out as well as on the way in, so a row
                // written by a newer build cannot make this one unrenderable.
                let user_device_type: Option<String> =
                    row.get::<_, Option<String>>(34)?.and_then(|raw| {
                        crate::discovery::DeviceType::parse_strict(&raw)
                            .map(|t| t.as_str().to_string())
                    });
                let evidence_state =
                    crate::discovery::freshness(row.get::<_, Option<i64>>(35)?.unwrap_or(0));
                let discovery = device_type.map(|device_type| InventoryDiscovery {
                    detected_name: detected_name.clone(),
                    device_type,
                    // Reduced where every claim behind it is stale, by the same
                    // rule the drawer uses. See `discovery::cap_for_freshness`.
                    type_confidence: crate::discovery::cap_for_freshness(
                        crate::discovery::Confidence::parse(
                            &row.get::<_, Option<String>>(28)
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| "unknown".into()),
                        ),
                        evidence_state,
                    )
                    .as_str()
                    .to_string(),
                    manufacturer: row.get(29).ok().flatten(),
                    model_name: row.get(30).ok().flatten(),
                    services: list_from_json(
                        row.get::<_, Option<String>>(31).ok().flatten().as_deref(),
                    ),
                    sources: list_from_json(
                        row.get::<_, Option<String>>(32).ok().flatten().as_deref(),
                    ),
                    last_discovered_at: row.get(33).ok().flatten(),
                    evidence_freshness: evidence_state.as_str().to_string(),
                });
                Ok(InventoryRow {
                    device_id,
                    network_scope_id: row.get(1)?,
                    network_name: row.get(2)?,
                    identity_source: parse_source(&row.get::<_, String>(3)?),
                    display_name: inventory::display_name_detected(
                        custom_name.as_deref(),
                        detected_name.as_deref(),
                        hostname.as_deref(),
                        vendor.as_deref(),
                        current_ip.as_deref().unwrap_or(""),
                    ),
                    custom_name,
                    hostname,
                    previous_ips: Vec::new(),
                    mac: row.get(7)?,
                    vendor,
                    os_guess: row.get(20)?,
                    status: DeviceStatus::parse(&row.get::<_, String>(11)?),
                    presence: if present {
                        PresenceState::Present
                    } else if comparable {
                        PresenceState::Missing
                    } else {
                        PresenceState::Unknown
                    },
                    first_seen: row.get(9)?,
                    last_seen: row.get(10)?,
                    last_completed_scan_id: row.get(21)?,
                    last_completed_scan_at: row.get(24)?,
                    observation_count: row.get(13)?,
                    open_ports: row
                        .get::<_, Option<String>>(15)?
                        .as_deref()
                        .map(parse_ports)
                        .unwrap_or_default(),
                    notes_present: row.get(12)?,
                    notes_excerpt: row.get(25)?,
                    latest_response_ms: row.get(16)?,
                    latest_icmp_ms: row.get(17)?,
                    latest_tcp_ms: row.get(18)?,
                    current_ip,
                    discovery,
                    user_device_type,
                })
            })
            .map_err(sql_err)?;

        let mut inventory_rows: Vec<InventoryRow> =
            rows.collect::<Result<_, _>>().map_err(sql_err)?;
        let mut present = 0i64;
        let mut missing = 0i64;
        let mut unknown = 0i64;
        let mut networks: HashMap<i64, (String, i64)> = HashMap::new();

        for row in &mut inventory_rows {
            match row.presence {
                PresenceState::Present => present += 1,
                PresenceState::Missing => missing += 1,
                PresenceState::Unknown => unknown += 1,
            }
            if let Some(history) = ip_history.remove(&row.device_id) {
                row.previous_ips = history
                    .into_iter()
                    .filter(|ip| Some(ip.as_str()) != row.current_ip.as_deref())
                    .take(PREVIOUS_IP_LIMIT)
                    .collect();
            }
            if let Some(scope) = row.network_scope_id {
                let entry = networks.entry(scope).or_insert_with(|| {
                    (
                        row.network_name.clone().unwrap_or_else(|| "Network".into()),
                        0,
                    )
                });
                entry.1 += 1;
            }
        }

        let mut networks: Vec<NetworkOption> = networks
            .into_iter()
            .map(|(id, (name, device_count))| NetworkOption {
                id,
                name,
                device_count,
            })
            .collect();
        // Case-insensitive, so the filter menu reads alphabetically whatever
        // capitalisation the operator used for a network name.
        networks.sort_by_key(|network| network.name.to_lowercase());

        let needs_completed_scan = !inventory_rows.is_empty()
            && inventory_rows
                .iter()
                .all(|r| r.last_completed_scan_id.is_none());

        Ok(InventorySummary {
            rows: inventory_rows,
            networks,
            present,
            missing,
            unknown,
            needs_completed_scan,
        })
    }

    /// The Changes inbox: every persisted change event, newest first.
    ///
    /// Names and network labels are resolved against the *current* device and
    /// scope rows, so renaming a device or a network updates the inbox without
    /// rewriting history. The label recorded with the event is the fallback for
    /// a device that has since been removed.
    pub fn change_events(&self) -> Result<ChangeFeed, String> {
        let conn = self.lock()?;
        let (total, unreviewed): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(state = 'unreviewed'), 0) FROM change_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql_err)?;

        let mut stmt = conn
            .prepare(
                "SELECT ce.id, ce.event_key, ce.scan_id, ce.baseline_scan_id, ce.network_scope_id,
                        ns.display_name, ce.device_id, ce.device_label, ce.ip, ce.mac, ce.vendor,
                        ce.change_type, ce.old_value, ce.new_value, ce.details, ce.state,
                        ce.created_at, ce.scan_at, ce.baseline_at, ce.acknowledged_at,
                        d.status, d.custom_name, d.hostname, d.vendor
                 FROM change_events ce
                 LEFT JOIN network_scopes ns ON ns.id = ce.network_scope_id
                 LEFT JOIN devices d ON d.id = ce.device_id
                 ORDER BY ce.id DESC
                 LIMIT ?1",
            )
            .map_err(sql_err)?;
        let events = stmt
            .query_map(params![CHANGE_EVENT_LIMIT], read_change_event)
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;

        Ok(ChangeFeed {
            truncated: total > events.len() as i64,
            events,
            unreviewed,
            total,
            starts_after_scan_id: changes_watermark(&conn)?,
        })
    }

    /// Move change events into a review state. One transaction, so a bulk
    /// acknowledgement either lands completely or not at all.
    ///
    /// Acknowledging stamps the time; moving back to unreviewed clears it, which
    /// is what makes Undo honest rather than leaving a stale timestamp behind.
    pub fn set_change_state(&self, ids: &[i64], state: ChangeState) -> Result<BulkOutcome, String> {
        if ids.is_empty() {
            return Ok(BulkOutcome {
                updated: 0,
                missing: Vec::new(),
            });
        }
        let mut conn = self.lock()?;
        let now = chrono::Local::now().to_rfc3339();
        let tx = conn.transaction().map_err(sql_err)?;
        let mut updated = 0usize;
        let mut missing = Vec::new();
        {
            let mut stmt = tx
                .prepare("UPDATE change_events SET state = ?1, acknowledged_at = ?2 WHERE id = ?3")
                .map_err(sql_err)?;
            let stamp = (state == ChangeState::Acknowledged).then_some(now);
            for id in ids {
                let count = stmt
                    .execute(params![state.as_str(), stamp, id])
                    .map_err(sql_err)?;
                if count == 0 {
                    missing.push(*id);
                } else {
                    updated += count;
                }
            }
        }
        tx.commit().map_err(sql_err)?;
        Ok(BulkOutcome { updated, missing })
    }

    /// Classify several devices at once, for the Inventory's bulk actions.
    ///
    /// Marking a device Ignored also takes its existing unreviewed changes out
    /// of the inbox, because leaving them there would contradict the action the
    /// operator just took. Nothing is deleted: the events keep their record and
    /// come back with the Ignored filter.
    pub fn set_device_statuses(
        &self,
        ids: &[i64],
        status: DeviceStatus,
    ) -> Result<BulkOutcome, String> {
        if ids.is_empty() {
            return Ok(BulkOutcome {
                updated: 0,
                missing: Vec::new(),
            });
        }
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(sql_err)?;
        let mut updated = 0usize;
        let mut missing = Vec::new();
        {
            let mut stmt = tx
                .prepare("UPDATE devices SET status = ?1 WHERE id = ?2")
                .map_err(sql_err)?;
            let mut hide = tx
                .prepare(
                    "UPDATE change_events SET state = 'ignored'
                     WHERE device_id = ?1 AND state = 'unreviewed'",
                )
                .map_err(sql_err)?;
            for id in ids {
                let count = stmt
                    .execute(params![status.as_str(), id])
                    .map_err(sql_err)?;
                if count == 0 {
                    missing.push(*id);
                    continue;
                }
                updated += count;
                if status == DeviceStatus::Ignored {
                    hide.execute(params![id]).map_err(sql_err)?;
                }
            }
        }
        tx.commit().map_err(sql_err)?;
        Ok(BulkOutcome { updated, missing })
    }

    pub fn device_detail(&self, id: i64) -> Result<DeviceDetail, String> {
        let conn = self.lock()?;
        let (device, network_name) = conn
            .query_row(
                "SELECT d.id, d.network_scope_id, d.identity_key, d.identity_source, d.mac,
                        d.custom_name, d.hostname, d.vendor, d.last_ip, d.first_seen,
                        d.last_seen, d.status, d.notes,
                        (SELECT COUNT(*) FROM hosts h WHERE h.device_id = d.id),
                        d.user_device_type,
                        ns.display_name
                 FROM devices d
                 LEFT JOIN network_scopes ns ON ns.id = d.network_scope_id
                 WHERE d.id = ?1",
                params![id],
                |row| Ok((read_device(row)?, row.get::<_, Option<String>>(15)?)),
            )
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| format!("Device {id} is no longer in the inventory."))?;

        let mut stmt = conn
            .prepare(
                "SELECT h.scan_id, s.target, h.last_seen, h.ip, h.hostname, h.vendor,
                        h.open_ports, h.response_ms, h.icmp_ms, h.tcp_ms, h.ttl, h.os_guess
                 FROM hosts h JOIN scans s ON s.id = h.scan_id
                 WHERE h.device_id = ?1
                 ORDER BY h.scan_id DESC
                 LIMIT ?2",
            )
            .map_err(sql_err)?;
        let observations = stmt
            .query_map(params![id, DEVICE_HISTORY_LIMIT], |row| {
                Ok(DeviceObservation {
                    scan_id: row.get(0)?,
                    scan_target: row.get(1)?,
                    observed_at: row.get(2)?,
                    ip: row.get(3)?,
                    hostname: row.get(4)?,
                    vendor: row.get(5)?,
                    open_ports: parse_ports(&row.get::<_, String>(6)?),
                    response_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                    icmp_ms: row.get(8)?,
                    tcp_ms: row.get(9)?,
                    ttl: row.get::<_, Option<i64>>(10)?.map(|v| v as u8),
                    os_guess: row.get(11)?,
                })
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;

        let mut previous_ips: Vec<String> = Vec::new();
        for obs in &observations {
            if !previous_ips.contains(&obs.ip) {
                previous_ips.push(obs.ip.clone());
            }
        }

        // Diff the two newest observations so the drawer can say what changed
        // without the caller loading a full comparison.
        let recent_changes = match observations.as_slice() {
            [newest, previous, ..] => {
                inventory::diff_fields(&observation_as_host(previous), &observation_as_host(newest))
            }
            _ => Vec::new(),
        };

        // The reviewable records for this device, so the drawer opened from the
        // Changes inbox can show what happened and when it was reviewed.
        let mut stmt = conn
            .prepare(
                "SELECT ce.id, ce.event_key, ce.scan_id, ce.baseline_scan_id, ce.network_scope_id,
                        ns.display_name, ce.device_id, ce.device_label, ce.ip, ce.mac, ce.vendor,
                        ce.change_type, ce.old_value, ce.new_value, ce.details, ce.state,
                        ce.created_at, ce.scan_at, ce.baseline_at, ce.acknowledged_at,
                        d.status, d.custom_name, d.hostname, d.vendor
                 FROM change_events ce
                 LEFT JOIN network_scopes ns ON ns.id = ce.network_scope_id
                 LEFT JOIN devices d ON d.id = ce.device_id
                 WHERE ce.device_id = ?1
                 ORDER BY ce.id DESC
                 LIMIT 30",
            )
            .map_err(sql_err)?;
        let events = stmt
            .query_map(params![id], read_change_event)
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;

        let presence = device_presence(&conn, id, device.network_scope_id)?;
        let discovery = read_device_discovery(&conn, id)?;

        Ok(DeviceDetail {
            device,
            observations,
            previous_ips,
            recent_changes,
            events,
            network_name,
            presence,
            discovery,
        })
    }

    pub fn set_device_name(&self, id: i64, name: Option<String>) -> Result<(), String> {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if let Some(n) = &name {
            if n.chars().count() > 120 {
                return Err("Device names are limited to 120 characters.".into());
            }
        }
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE devices SET custom_name = ?1 WHERE id = ?2",
                params![name, id],
            )
            .map_err(sql_err)?;
        if updated == 0 {
            return Err(format!("Device {id} is no longer in the inventory."));
        }
        Ok(())
    }

    /// Set, change or clear the operator's device-type correction.
    ///
    /// `None` clears it, which restores Auto and reveals whatever ArcScan
    /// currently detects — not what it detected when the override was set, so a
    /// device that has since started advertising properly shows its new answer
    /// immediately.
    ///
    /// `Some("unknown")` is an explicit choice and is stored as one. It is not
    /// the same as clearing: a person who has looked at a device and concluded
    /// that neither they nor ArcScan can say what it is has recorded something
    /// real, and the next scan must not talk them out of it.
    ///
    /// # What this deliberately does not touch
    ///
    /// One column on one row. Not `identity_key`, `identity_source`, `mac`,
    /// `network_scope_id`, `first_seen`, `last_seen`, `status`, `custom_name`
    /// or `notes`; not `device_discovery`; not `discovery_evidence`; not
    /// `hosts`; and not `change_events`. A correction to what ArcScan calls a
    /// device is not an event on the network, and writing one must not create
    /// one.
    pub fn set_device_type_override(
        &self,
        id: i64,
        device_type: Option<String>,
    ) -> Result<(), String> {
        // Validated against the shipped vocabulary before it reaches SQL. The
        // strict parser is used rather than the forgiving one on purpose: a
        // value that is not a type must be an error, because the forgiving
        // parser would turn a typo into an explicit choice of Unknown, which is
        // itself a meaningful answer nobody made.
        let stored = match device_type {
            None => None,
            Some(raw) => Some(
                crate::discovery::DeviceType::parse_strict(raw.trim())
                    .ok_or_else(|| format!("{raw:?} is not a device type ArcScan recognises."))?
                    .as_str()
                    .to_string(),
            ),
        };
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE devices SET user_device_type = ?1 WHERE id = ?2",
                params![stored, id],
            )
            .map_err(sql_err)?;
        if updated == 0 {
            return Err(format!("Device {id} is no longer in the inventory."));
        }
        Ok(())
    }

    /// Build the redacted discovery report for one device.
    ///
    /// The query below is the privacy guarantee, and it is worth reading as
    /// one: it selects the device's vendor, its address and its type override,
    /// and nothing else from `devices`. `notes`, `mac`, `identity_key` and
    /// `custom_name` are not in it. The evidence query selects no serial, and
    /// [`crate::discovery::diagnostics`] drops identifier-bearing kinds a
    /// second time regardless.
    ///
    /// Nothing is written, nothing is sent, and no file is created. The caller
    /// puts the returned string on the clipboard.
    pub fn device_discovery_report(&self, id: i64, app_version: &str) -> Result<String, String> {
        use crate::discovery::diagnostics::{DeviceDiagnostic, DiagnosticEvidence};

        let conn = self.lock()?;
        let (vendor, last_ip, user_override) = conn
            .query_row(
                "SELECT vendor, last_ip, user_device_type FROM devices WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?
            .ok_or_else(|| format!("Device {id} is no longer in the inventory."))?;

        let record = read_device_discovery(&conn, id)?;

        let detected_type = crate::discovery::DeviceType::parse(
            record
                .as_ref()
                .map(|r| r.device_type.as_str())
                .unwrap_or("unknown"),
        );
        let detected_confidence = crate::discovery::Confidence::parse(
            record
                .as_ref()
                .map(|r| r.type_confidence.as_str())
                .unwrap_or("unknown"),
        );
        let user_type = user_override
            .as_deref()
            .and_then(crate::discovery::DeviceType::parse_strict);
        let resolved =
            crate::discovery::effective_type(detected_type, detected_confidence, user_type);

        // The most recent scan that reached this device, and how its discovery
        // pass went. A count of scans and four words; no target, no address.
        let quality: Option<crate::discovery::DiscoveryQuality> = conn
            .query_row(
                "SELECT s.discovery_summary FROM hosts h
                 JOIN scans s ON s.id = h.scan_id
                 WHERE h.device_id = ?1
                 ORDER BY h.scan_id DESC LIMIT 1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(sql_err)?
            .flatten()
            .and_then(|raw| serde_json::from_str::<crate::discovery::DiscoveryReport>(&raw).ok())
            .map(|report| report.quality());

        let evidence: Vec<DiagnosticEvidence> = record
            .as_ref()
            .map(|r| {
                r.evidence
                    .iter()
                    .map(|e| DiagnosticEvidence {
                        source: e.source.clone(),
                        kind: e.kind.clone(),
                        value: e.value.clone(),
                        freshness: crate::discovery::Freshness::parse(&e.freshness),
                        misses: e.misses,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let empty: Vec<String> = Vec::new();
        Ok(crate::discovery::diagnostics::build_report(
            &DeviceDiagnostic {
                app_version,
                effective_type: resolved.effective_type,
                type_source: Some(resolved.type_source),
                detected_type: resolved.detected_type,
                detected_confidence: resolved.detected_confidence,
                detected_name: record.as_ref().and_then(|r| r.detected_name.as_deref()),
                manufacturer: record.as_ref().and_then(|r| r.manufacturer.as_deref()),
                model: record.as_ref().and_then(|r| r.model_name.as_deref()),
                oui_vendor: vendor.as_deref(),
                sources: record
                    .as_ref()
                    .map(|r| r.sources.as_slice())
                    .unwrap_or(&empty),
                services: record
                    .as_ref()
                    .map(|r| r.services.as_slice())
                    .unwrap_or(&empty),
                evidence: &evidence,
                discovery_quality: quality,
                ip: last_ip.as_deref(),
                // Not derivable from stored state without re-reading the
                // interface, and reporting a stale answer would be worse than
                // reporting none.
                is_gateway: false,
            },
        ))
    }

    pub fn set_device_status(&self, id: i64, status: DeviceStatus) -> Result<(), String> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE devices SET status = ?1 WHERE id = ?2",
                params![status.as_str(), id],
            )
            .map_err(sql_err)?;
        if updated == 0 {
            return Err(format!("Device {id} is no longer in the inventory."));
        }
        Ok(())
    }

    /// Note bodies for a set of devices, for an export.
    ///
    /// The inventory query carries only whether a note exists, because the table
    /// shows an indicator; an export needs the text, so it is fetched once for
    /// exactly the devices being written rather than loaded into every row.
    pub fn device_notes(&self, ids: &[i64]) -> Result<Vec<(i64, String)>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT id, notes FROM devices WHERE id = ?1 AND notes IS NOT NULL")
            .map_err(sql_err)?;
        let mut out = Vec::new();
        for id in ids {
            let row: Option<(i64, String)> = stmt
                .query_row(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(sql_err)?;
            if let Some(pair) = row {
                out.push(pair);
            }
        }
        Ok(out)
    }

    pub fn set_device_notes(&self, id: i64, notes: Option<String>) -> Result<(), String> {
        let notes = notes.filter(|n| !n.trim().is_empty());
        if let Some(n) = &notes {
            if n.chars().count() > 4_000 {
                return Err("Device notes are limited to 4,000 characters.".into());
            }
        }
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE devices SET notes = ?1 WHERE id = ?2",
                params![notes, id],
            )
            .map_err(sql_err)?;
        if updated == 0 {
            return Err(format!("Device {id} is no longer in the inventory."));
        }
        Ok(())
    }

    /// Adopt device labels that v1.6 kept in browser local storage, keyed by
    /// MAC. Only fills gaps: a name already set in the database wins, so running
    /// the import twice cannot overwrite an edit made since.
    pub fn import_device_labels(&self, labels: HashMap<String, String>) -> Result<usize, String> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(sql_err)?;
        let mut adopted = 0usize;
        for (mac, label) in labels {
            let Some(mac) = inventory::normalize_mac(&mac) else {
                continue;
            };
            let label = label.trim();
            // v1.6 stored an empty label to mean "starred but unnamed", which
            // maps onto the Known status with no custom name.
            let name: Option<&str> = (!label.is_empty()).then_some(label);
            let changed = tx
                .execute(
                    "UPDATE devices
                     SET custom_name = COALESCE(custom_name, ?1),
                         status = CASE WHEN status = 'unclassified' THEN 'known' ELSE status END
                     WHERE mac = ?2",
                    params![name, mac],
                )
                .map_err(sql_err)?;
            adopted += changed;
        }
        tx.commit().map_err(sql_err)?;
        Ok(adopted)
    }

    /// IPs of the most recent saved scan. Retained so exports and older callers
    /// keep working; change detection uses [`Db::compare_scan`] instead, because
    /// comparing by IP alone is what produced false new-device results.
    pub fn last_scan_ips(&self) -> Result<Vec<String>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT h.ip FROM hosts h
                 WHERE h.scan_id = (SELECT id FROM scans ORDER BY id DESC LIMIT 1)",
            )
            .map_err(sql_err)?;
        let ips = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        Ok(ips)
    }
}

/// Drop change events' references to scans that no longer exist.
///
/// `change_events.scan_id` is deliberately not a foreign key — the record has to
/// outlive the scan that produced it — but that means nothing nulls it
/// automatically, and an id pointing at a deleted scan would make the inbox's
/// "Open the scan" fail rather than saying the scan is gone. The event keeps its
/// own copy of the scan dates and the device label, so it stays readable.
fn clear_deleted_scan_links(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute(
        "UPDATE change_events
         SET scan_id = CASE
                 WHEN scan_id IN (SELECT id FROM scans) THEN scan_id ELSE NULL END,
             baseline_scan_id = CASE
                 WHEN baseline_scan_id IN (SELECT id FROM scans) THEN baseline_scan_id
                 ELSE NULL END
         WHERE (scan_id IS NOT NULL AND scan_id NOT IN (SELECT id FROM scans))
            OR (baseline_scan_id IS NOT NULL
                AND baseline_scan_id NOT IN (SELECT id FROM scans))",
        [],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Presence for a single device, applying exactly the rules documented on
/// [`PresenceState`] and implemented set-wise in [`INVENTORY_SQL`].
///
/// Kept as its own function rather than reusing the inventory query because the
/// drawer asks about one device, and a test that both agree is cheaper than a
/// second implementation drifting.
fn device_presence(
    conn: &Connection,
    device_id: i64,
    scope_id: Option<i64>,
) -> Result<PresenceState, String> {
    let Some(scope) = scope_id else {
        return Ok(PresenceState::Unknown);
    };
    let reference: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, target_key, coverage_key FROM scans
             WHERE network_scope_id = ?1 AND status = 'completed'
               AND coverage_key <> '' AND coverage_key NOT LIKE 'legacy:%'
             ORDER BY id DESC LIMIT 1",
            params![scope],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql_err)?;
    let Some((scan_id, target_key, coverage_key)) = reference else {
        return Ok(PresenceState::Unknown);
    };

    let present: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE device_id = ?1 AND scan_id = ?2)",
            params![device_id, scan_id],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    if present {
        return Ok(PresenceState::Present);
    }

    let comparable: bool = conn
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hosts h JOIN scans s ON s.id = h.scan_id
                 WHERE h.device_id = ?1 AND s.network_scope_id = ?2
                   AND s.status = 'completed'
                   AND s.target_key = ?3 AND s.coverage_key = ?4)",
            params![device_id, scope, target_key, coverage_key],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    Ok(if comparable {
        PresenceState::Missing
    } else {
        PresenceState::Unknown
    })
}

/// The Inventory query.
///
/// Four common table expressions do the work that would otherwise be a query per
/// row:
///
/// * `reference` — each network scope's most recent scan that both completed and
///   recorded which ports it checked. This is the only scan presence is decided
///   from, which is what keeps a stopped scan from marking anything Missing and
///   a pre-coverage-key scan from being compared against.
/// * `present` — devices that scan actually saw.
/// * `comparable` — devices seen by *any* completed scan with the same target
///   and coverage as the reference. Absence only means something for these; for
///   anything else the reference scan was not looking in the same place.
/// * `latest` — the newest observation per device, in one window-function pass
///   over `hosts` rather than a lookup per device.
///
/// Notes are reduced to a boolean here on purpose: the table only ever shows an
/// indicator, and loading thousands of note bodies to render a dot would be
/// wasted work.
const INVENTORY_SQL: &str = r#"
WITH reference AS (
    SELECT s.network_scope_id AS scope_id, s.id AS scan_id, s.created_at AS created_at,
           s.target_key AS target_key, s.coverage_key AS coverage_key
    FROM scans s
    WHERE s.status = 'completed'
      AND s.coverage_key <> ''
      AND s.coverage_key NOT LIKE 'legacy:%'
      AND s.id = (
          SELECT MAX(s2.id) FROM scans s2
          WHERE s2.network_scope_id IS s.network_scope_id
            AND s2.status = 'completed'
            AND s2.coverage_key <> ''
            AND s2.coverage_key NOT LIKE 'legacy:%'
      )
),
present AS (
    SELECT DISTINCT h.device_id AS device_id
    FROM hosts h JOIN reference r ON r.scan_id = h.scan_id
    WHERE h.device_id IS NOT NULL
),
comparable AS (
    SELECT DISTINCT h.device_id AS device_id
    FROM hosts h
    JOIN scans s ON s.id = h.scan_id
    JOIN reference r ON r.scope_id IS s.network_scope_id
    WHERE h.device_id IS NOT NULL
      AND s.status = 'completed'
      AND s.target_key = r.target_key
      AND s.coverage_key = r.coverage_key
),
latest AS (
    SELECT device_id, ip, open_ports, response_ms, icmp_ms, tcp_ms, last_seen, os_guess
    FROM (
        SELECT h.device_id AS device_id, h.ip AS ip, h.open_ports AS open_ports,
               h.response_ms AS response_ms, h.icmp_ms AS icmp_ms, h.tcp_ms AS tcp_ms,
               h.last_seen AS last_seen, h.os_guess AS os_guess,
               ROW_NUMBER() OVER (PARTITION BY h.device_id ORDER BY h.scan_id DESC, h.id DESC) AS rn
        FROM hosts h
        WHERE h.device_id IS NOT NULL
    )
    WHERE rn = 1
),
counts AS (
    SELECT h.device_id AS device_id, COUNT(*) AS n
    FROM hosts h WHERE h.device_id IS NOT NULL GROUP BY h.device_id
),
-- How current each device's freshest discovery claim is, as one grouped pass
-- over the evidence table rather than a query per row.
--
-- The filter is the same one `write_discovery` uses to decide what may age, and
-- it has to be: a claim that is exempt from aging sits at zero misses forever,
-- and including it in a MIN would pin every device at "current" whatever else
-- went quiet. So the aggregate covers exactly the claims an ordinary mDNS and
-- SSDP pass re-hears without fetching a description — services, declared device
-- types, advertised names, host names, addresses.
--
-- A device with no such claim has no row here and reads as current, which is
-- right: nothing has been observed to go quiet.
freshness AS (
    SELECT device_id, MIN(misses) AS best_misses
    FROM discovery_evidence
    WHERE source IN ('mdns', 'ssdp')
      AND kind NOT IN ('manufacturer', 'model', 'model_number', 'serial_number', 'url')
    GROUP BY device_id
)
SELECT d.id, d.network_scope_id, ns.display_name, d.identity_source, d.custom_name, d.hostname,
       d.vendor, d.mac, d.last_ip, d.first_seen, d.last_seen, d.status,
       (d.notes IS NOT NULL AND TRIM(d.notes) <> '') AS has_notes,
       COALESCE(c.n, 0) AS observations,
       l.ip, l.open_ports, l.response_ms, l.icmp_ms, l.tcp_ms, l.last_seen, l.os_guess,
       r.scan_id,
       (p.device_id IS NOT NULL) AS is_present,
       (cmp.device_id IS NOT NULL) AS is_comparable,
       r.created_at,
       SUBSTR(d.notes, 1, 160) AS notes_excerpt,
       dd.detected_name, dd.device_type, dd.type_confidence, dd.manufacturer, dd.model_name,
       dd.services, dd.sources, dd.last_discovered_at,
       d.user_device_type, f.best_misses
FROM devices d
LEFT JOIN network_scopes ns ON ns.id = d.network_scope_id
LEFT JOIN reference r ON r.scope_id IS d.network_scope_id
LEFT JOIN latest l ON l.device_id = d.id
LEFT JOIN counts c ON c.device_id = d.id
LEFT JOIN present p ON p.device_id = d.id
LEFT JOIN comparable cmp ON cmp.device_id = d.id
-- One row per device by construction (device_id is the primary key), so this
-- join cannot multiply the result set the way a join onto the evidence table
-- would.
LEFT JOIN device_discovery dd ON dd.device_id = d.id
-- Grouped above, so this is one row per device too.
LEFT JOIN freshness f ON f.device_id = d.id
ORDER BY d.last_seen DESC, d.id DESC
"#;

/// Read one change-event row, resolving the device label against the device's
/// current name so a rename shows up everywhere at once.
fn read_change_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeEvent> {
    let ip: Option<String> = row.get(8)?;
    let stored_label: String = row.get(7)?;
    let device_status: Option<String> = row.get(20)?;
    let current_name: Option<String> = row.get(21)?;
    let current_hostname: Option<String> = row.get(22)?;
    let current_vendor: Option<String> = row.get(23)?;
    let device_label = if device_status.is_some() {
        inventory::display_name(
            current_name.as_deref(),
            current_hostname.as_deref(),
            current_vendor.as_deref(),
            ip.as_deref().unwrap_or(&stored_label),
        )
    } else {
        stored_label
    };
    let details: Option<String> = row.get(14)?;
    let (opened_ports, closed_ports) = parse_port_details(details.as_deref());
    Ok(ChangeEvent {
        id: row.get(0)?,
        event_key: row.get(1)?,
        scan_id: row.get(2)?,
        baseline_scan_id: row.get(3)?,
        network_scope_id: row.get(4)?,
        network_name: row.get(5)?,
        device_id: row.get(6)?,
        device_label,
        mac: row.get(9)?,
        vendor: row.get(10)?,
        // An unrecognised type can only come from a newer build writing into
        // this database; showing it as a plain change beats hiding it.
        change_type: ChangeType::parse(&row.get::<_, String>(11)?)
            .unwrap_or(ChangeType::PortsChanged),
        old_value: row.get(12)?,
        new_value: row.get(13)?,
        opened_ports,
        closed_ports,
        state: ChangeState::parse(&row.get::<_, String>(15)?).unwrap_or_default(),
        created_at: row.get(16)?,
        scan_at: row.get(17)?,
        baseline_at: row.get(18)?,
        acknowledged_at: row.get(19)?,
        device_status: device_status.as_deref().map(DeviceStatus::parse),
        ip,
    })
}

/// Unpack the structured opened/closed port lists stored with a port change.
fn parse_port_details(details: Option<&str>) -> (Vec<u16>, Vec<u16>) {
    let Some(raw) = details else {
        return (Vec::new(), Vec::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (Vec::new(), Vec::new());
    };
    let list = |key: &str| -> Vec<u16> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_u64())
                    .filter_map(|n| u16::try_from(n).ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    (list("opened"), list("closed"))
}

/// Write one normalized change event per device, per kind of change, for a
/// completed scan that had a baseline to compare against.
///
/// # Uniqueness
///
/// `event_key` is `s{scan}|{device}|{type}`, derived entirely from data that
/// cannot change after the fact, and the table has a unique index on it. Saving
/// the same scan twice, retrying an interrupted save, or re-opening a scan
/// therefore cannot produce a second copy of a change: the insert conflicts and
/// does nothing, leaving the operator's existing review state alone.
///
/// # Ignored devices
///
/// A device the operator marked Ignored still gets its events recorded — the
/// history is never thrown away — but they are written already in the `ignored`
/// state, so they stay out of the default inbox and can be filtered back in.
fn record_change_events(
    tx: &Transaction<'_>,
    scan_id: i64,
    scope_id: i64,
    baseline: &BaselineScan,
    comparison: &ScanComparison,
    now: &str,
) -> Result<usize, String> {
    use std::collections::HashSet;

    // One query rather than a status lookup per changed device.
    let ignored: HashSet<i64> = {
        let mut stmt = tx
            .prepare("SELECT id FROM devices WHERE network_scope_id = ?1 AND status = 'ignored'")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![scope_id], |r| r.get::<_, i64>(0))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows.into_iter().collect()
    };

    let mut stmt = tx
        .prepare(
            "INSERT INTO change_events
                (event_key, scan_id, baseline_scan_id, network_scope_id, device_id, device_label,
                 ip, mac, vendor, change_type, old_value, new_value, details, state, created_at,
                 scan_at, baseline_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16)
             ON CONFLICT(event_key) DO NOTHING",
        )
        .map_err(sql_err)?;

    let mut written = 0usize;
    let mut write = |diff: &inventory::DeviceDiff,
                     kind: ChangeType,
                     old: Option<String>,
                     new: Option<String>,
                     details: Option<String>|
     -> Result<(), String> {
        let subject = match diff.device_id {
            Some(id) => format!("d{id}"),
            // A device the inventory could not identify still gets a stable key
            // from its address, so a retry does not duplicate it either.
            None => format!("ip:{}", diff.ip),
        };
        let state = match diff.device_id {
            Some(id) if ignored.contains(&id) => ChangeState::Ignored,
            _ => ChangeState::Unreviewed,
        };
        let count = stmt
            .execute(params![
                format!("s{scan_id}|{subject}|{}", kind.as_str()),
                scan_id,
                baseline.id,
                scope_id,
                diff.device_id,
                diff.name,
                diff.ip,
                diff.mac,
                diff.vendor,
                kind.as_str(),
                old,
                new,
                details,
                state.as_str(),
                now,
                baseline.created_at,
            ])
            .map_err(sql_err)?;
        written += count;
        Ok(())
    };

    for diff in &comparison.added {
        let kind = match diff.kind {
            ChangeKind::Returned => ChangeType::DeviceReturned,
            _ => ChangeType::DeviceAdded,
        };
        write(diff, kind, None, Some(diff.ip.clone()), None)?;
    }
    for diff in &comparison.removed {
        write(
            diff,
            ChangeType::DeviceMissing,
            Some(diff.ip.clone()),
            None,
            None,
        )?;
    }
    for diff in &comparison.changed {
        for field in &diff.fields {
            let Some(kind) = ChangeType::for_field(&field.field) else {
                continue;
            };
            // Port changes carry the structured lists, not only the display
            // text, so an export or a later UI can work with the numbers.
            let details = if kind == ChangeType::PortsChanged {
                serde_json::to_string(&serde_json::json!({
                    "opened": field.added_ports,
                    "closed": field.removed_ports,
                }))
                .ok()
            } else {
                None
            };
            write(diff, kind, field.from.clone(), field.to.clone(), details)?;
        }
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Discovery persistence (v1.8.2)
// ---------------------------------------------------------------------------

/// A device's discovery facts, reduced to the ones worth telling someone about
/// when they change. Everything omitted here — TTLs, cache lifetimes, boot ids,
/// header banners, protocol identifiers, the order a device listed things in —
/// is deliberately outside the comparison, because none of it means anything
/// changed about the device.
#[derive(Debug, Clone, Default, PartialEq)]
struct DiscoverySnapshot {
    detected_name: Option<String>,
    name_is_strong: bool,
    device_type: String,
    type_confidence: String,
    manufacturer: Option<String>,
    model_name: Option<String>,
    /// Currently-advertised services, sorted.
    services: Vec<String>,
    /// Which generation of the naming rules produced the name and model above.
    /// A snapshot older than [`NAMING_RULES_VERSION`] has its name and model
    /// compared silently exactly once. See that constant.
    naming_rules_version: i64,
}

fn json_list(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".into())
}

fn list_from_json(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|r| serde_json::from_str::<Vec<String>>(r).ok())
        .unwrap_or_default()
}

/// What one scan's discovery pass managed, as the two questions the aging rules
/// actually ask.
///
/// A pair rather than two booleans in a signature, because the two are always
/// read together and the wrong order would be silently wrong.
#[derive(Debug, Clone, Copy)]
struct DiscoveryPass {
    /// Both protocols ran to completion and the scan was not stopped. False for
    /// a partial scan, a stopped scan, a remote scan, a scan with discovery
    /// switched off, and a scan with no eligible local interface. Nothing ages
    /// unless this is true.
    full: bool,
    /// At least one description document was read, so the fields only a
    /// description carries had a fair chance to be re-observed.
    descriptions: bool,
}

/// Read what a device's discovery record said before this scan touched it.
fn read_discovery_snapshot(
    tx: &Transaction<'_>,
    device_id: i64,
) -> Result<Option<DiscoverySnapshot>, String> {
    let row = tx
        .query_row(
            "SELECT detected_name, name_source, device_type, type_confidence, manufacturer,
                    model_name, services, naming_rules_version
             FROM device_discovery WHERE device_id = ?1",
            params![device_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql_err)?;
    Ok(row.map(
        |(
            detected_name,
            name_source,
            device_type,
            type_confidence,
            manufacturer,
            model_name,
            services,
            naming_rules_version,
        )| {
            DiscoverySnapshot {
                name_is_strong: detected_name.is_some()
                    && matches!(name_source.as_deref(), Some("mdns") | Some("ssdp")),
                detected_name,
                device_type,
                type_confidence,
                manufacturer,
                model_name,
                services: list_from_json(services.as_deref()),
                naming_rules_version,
            }
        },
    ))
}

/// Record one device's discovery evidence and rebuild its resolved record.
///
/// `pass` says what this scan's discovery actually managed, which is what
/// decides whether a claim it did not re-hear counts as missed.
fn write_discovery(
    tx: &Transaction<'_>,
    device_id: i64,
    scope_id: i64,
    scan_id: i64,
    discovery: &crate::scanner::HostDiscovery,
    now: &str,
    pass: DiscoveryPass,
) -> Result<DiscoverySnapshot, String> {
    let previous = read_discovery_snapshot(tx, device_id)?;

    // Evidence first. `first_seen` survives every re-observation, which is what
    // makes "advertising this since March" a fact rather than a guess.
    let mut upsert = tx
        .prepare(
            "INSERT INTO discovery_evidence
                (device_id, network_scope_id, source, kind, key, value, normalized_value,
                 confidence, first_seen, last_seen, last_scan_id, misses)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, 0)
             ON CONFLICT(device_id, source, kind, key, normalized_value) DO UPDATE SET
                 value        = excluded.value,
                 confidence   = excluded.confidence,
                 last_seen    = excluded.last_seen,
                 last_scan_id = excluded.last_scan_id,
                 misses       = 0",
        )
        .map_err(sql_err)?;

    let mut record = |source: &str, kind: &str, key: &str, value: &str, confidence: &str| {
        let normalized = crate::discovery::model::normalize_value(value);
        if normalized.is_empty() {
            return Ok(());
        }
        upsert
            .execute(params![
                device_id, scope_id, source, kind, key, value, normalized, confidence, now,
                scan_id,
            ])
            .map(|_| ())
            .map_err(sql_err)
    };

    let source = discovery.name_source.as_deref().unwrap_or("mdns");
    if let Some(name) = &discovery.detected_name {
        record(source, "display_name", "", name, "high")?;
    }
    for alternate in &discovery.alternate_names {
        record(source, "display_name", "alternate", alternate, "medium")?;
    }
    for (kind, value) in [
        ("manufacturer", &discovery.manufacturer),
        ("model", &discovery.model_name),
        ("model_number", &discovery.model_number),
        ("serial_number", &discovery.serial_number),
        ("hostname", &discovery.mdns_hostname),
        ("url", &discovery.presentation_url),
    ] {
        if let Some(value) = value {
            record("ssdp", kind, "", value, "high")?;
        }
    }
    for service in &discovery.services {
        record("mdns", "service", service, service, "high")?;
    }
    for address in &discovery.ipv6_addresses {
        record("mdns", "ipv6_address", "", address, "medium")?;
    }
    drop(upsert);

    // Absence bookkeeping. A claim this scan did not re-hear gets one miss; a
    // claim it did hear had its counter reset to zero by the upsert above.
    //
    // v1.8.2 counted misses for advertised services only, to decide when a
    // service had gone. v1.8.3 counts them for every claim mDNS or SSDP made,
    // because the same question — "has anything confirmed this lately?" —
    // decides whether a manufacturer, a model or a type declaration is still
    // worth leaning on. The counter is the same column and the same increment;
    // only the set of rows it applies to grew.
    //
    // What is *not* aged, and why:
    //
    // * anything from a source that is not mDNS or SSDP. A reverse-DNS name or
    //   an OUI manufacturer is not something a discovery pass listens for, so a
    //   discovery pass hearing nothing says nothing about it.
    // * every claim, on a scan that was not a completed, uninterrupted,
    //   both-protocols pass. `full_discovery` is the caller's answer to that,
    //   and it is already false for a partial scan, a stopped scan, a remote
    //   scan, a scan with discovery switched off and a scan with no eligible
    //   interface.
    // * a device the scan did not find. The caller only reaches this function
    //   for a device this scan observed, so a missing device's evidence is
    //   never touched — a device that was switched off has not stopped
    //   advertising.
    // * description-derived claims, when descriptions were not fetched. Reading
    //   a description document is a separate setting and a separate budget, and
    //   a pass that never asked for one has not observed its absence.
    if pass.full {
        tx.execute(
            "UPDATE discovery_evidence SET misses = misses + 1
             WHERE device_id = ?1
               AND (last_scan_id IS NULL OR last_scan_id <> ?2)
               AND source IN ('mdns', 'ssdp')
               AND (?3 = 1 OR kind NOT IN
                    ('manufacturer', 'model', 'model_number', 'serial_number', 'url'))",
            params![device_id, scan_id, pass.descriptions as i64],
        )
        .map_err(sql_err)?;
    }

    // Services still believed present: heard this scan, or not yet missed often
    // enough to be disbelieved.
    let services: Vec<String> = {
        let mut stmt = tx
            .prepare(
                "SELECT value FROM discovery_evidence
                 WHERE device_id = ?1 AND kind = 'service' AND misses < ?2
                 ORDER BY value ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![device_id, SERVICE_ABSENCE_MISSES], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };

    // A settled, high-confidence type is not surrendered to a weaker reading.
    let fresh = classification_from(
        &discovery.device_type,
        &discovery.type_confidence,
        &discovery.type_evidence,
    );
    let settled = previous.as_ref().map(|p| {
        classification_from(
            &Some(p.device_type.clone()),
            &Some(p.type_confidence.clone()),
            &[],
        )
    });
    let resolved = crate::discovery::classify::reconcile(settled.as_ref(), fresh);

    let snapshot = DiscoverySnapshot {
        detected_name: discovery.detected_name.clone(),
        name_is_strong: discovery.detected_name.is_some(),
        device_type: resolved.device_type.as_str().to_string(),
        type_confidence: resolved.confidence.as_str().to_string(),
        manufacturer: discovery.manufacturer.clone(),
        model_name: discovery.model_name.clone(),
        services: services.clone(),
        naming_rules_version: NAMING_RULES_VERSION,
    };

    tx.execute(
        "INSERT INTO device_discovery
            (device_id, network_scope_id, detected_name, name_source, device_type,
             type_confidence, type_evidence, type_conflicts, manufacturer, model_name,
             model_number, serial_number, mdns_hostname, ssdp_friendly_name, services,
             sources, alternate_names, ipv6_addresses, presentation_url,
             first_discovered_at, last_discovered_at, naming_rules_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?20, ?21)
         ON CONFLICT(device_id) DO UPDATE SET
             network_scope_id   = excluded.network_scope_id,
             detected_name      = COALESCE(excluded.detected_name, device_discovery.detected_name),
             name_source        = COALESCE(excluded.name_source, device_discovery.name_source),
             device_type        = excluded.device_type,
             type_confidence    = excluded.type_confidence,
             type_evidence      = excluded.type_evidence,
             type_conflicts     = excluded.type_conflicts,
             manufacturer       = COALESCE(excluded.manufacturer, device_discovery.manufacturer),
             model_name         = COALESCE(excluded.model_name, device_discovery.model_name),
             model_number       = COALESCE(excluded.model_number, device_discovery.model_number),
             serial_number      = COALESCE(excluded.serial_number, device_discovery.serial_number),
             mdns_hostname      = COALESCE(excluded.mdns_hostname, device_discovery.mdns_hostname),
             ssdp_friendly_name = COALESCE(excluded.ssdp_friendly_name,
                                           device_discovery.ssdp_friendly_name),
             services           = excluded.services,
             sources            = excluded.sources,
             alternate_names    = excluded.alternate_names,
             ipv6_addresses     = excluded.ipv6_addresses,
             presentation_url   = COALESCE(excluded.presentation_url,
                                           device_discovery.presentation_url),
             last_discovered_at = excluded.last_discovered_at,
             naming_rules_version = excluded.naming_rules_version",
        params![
            device_id,
            scope_id,
            discovery.detected_name,
            discovery.name_source,
            snapshot.device_type,
            snapshot.type_confidence,
            json_list(&resolved.evidence),
            json_list(&discovery.type_conflicts),
            discovery.manufacturer,
            discovery.model_name,
            discovery.model_number,
            discovery.serial_number,
            discovery.mdns_hostname,
            discovery.ssdp_friendly_name,
            json_list(&services),
            json_list(&discovery.sources),
            json_list(&discovery.alternate_names),
            json_list(&discovery.ipv6_addresses),
            discovery.presentation_url,
            now,
            NAMING_RULES_VERSION,
        ],
    )
    .map_err(sql_err)?;

    Ok(snapshot)
}

/// Load one device's full discovery record, evidence included.
///
/// The evidence rows are ordered strongest-and-most-recent first and capped:
/// the drawer shows a summary, not an audit log, and a device that has been on
/// the network for years should still open instantly.
fn read_device_discovery(
    conn: &Connection,
    device_id: i64,
) -> Result<Option<DeviceDiscovery>, String> {
    let record = conn
        .query_row(
            "SELECT detected_name, name_source, device_type, type_confidence, type_evidence,
                    type_conflicts, manufacturer, model_name, model_number, serial_number,
                    mdns_hostname, ssdp_friendly_name, services, sources, alternate_names,
                    ipv6_addresses, presentation_url, first_discovered_at, last_discovered_at
             FROM device_discovery WHERE device_id = ?1",
            params![device_id],
            |r| {
                Ok(DeviceDiscovery {
                    detected_name: r.get(0)?,
                    name_source: r.get(1)?,
                    device_type: r.get(2)?,
                    type_confidence: r.get(3)?,
                    type_evidence: list_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
                    type_conflicts: list_from_json(r.get::<_, Option<String>>(5)?.as_deref()),
                    manufacturer: r.get(6)?,
                    model_name: r.get(7)?,
                    model_number: r.get(8)?,
                    serial_number: r.get(9)?,
                    mdns_hostname: r.get(10)?,
                    ssdp_friendly_name: r.get(11)?,
                    services: list_from_json(r.get::<_, Option<String>>(12)?.as_deref()),
                    sources: list_from_json(r.get::<_, Option<String>>(13)?.as_deref()),
                    alternate_names: list_from_json(r.get::<_, Option<String>>(14)?.as_deref()),
                    ipv6_addresses: list_from_json(r.get::<_, Option<String>>(15)?.as_deref()),
                    presentation_url: r.get(16)?,
                    first_discovered_at: r.get(17)?,
                    last_discovered_at: r.get(18)?,
                    evidence: Vec::new(),
                    // Both filled in below, once the evidence has been read.
                    evidence_freshness: String::new(),
                    raw_type_confidence: String::new(),
                })
            },
        )
        .optional()
        .map_err(sql_err)?;

    let Some(mut record) = record else {
        return Ok(None);
    };

    // Ordered so the drawer's first screenful is the evidence that still counts:
    // current claims before aging ones before stale ones, and inside each group
    // the most recently seen first. A device that has been on the network for
    // years still opens instantly, because the list is capped either way.
    let mut stmt = conn
        .prepare(
            "SELECT source, kind, key, value, confidence, first_seen, last_seen, misses
             FROM discovery_evidence
             WHERE device_id = ?1
             ORDER BY misses ASC, last_seen DESC, kind ASC, source ASC, value ASC
             LIMIT 60",
        )
        .map_err(sql_err)?;
    record.evidence = stmt
        .query_map(params![device_id], |r| {
            // Source, kind and confidence are normalized through their own
            // vocabularies on the way out. A value written by a newer build, or
            // corrupted, becomes a known word rather than an unrecognised
            // string the interface would have no label for.
            let source: String = r.get(0)?;
            let kind: String = r.get(1)?;
            let confidence: String = r.get(4)?;
            let misses: i64 = r.get(7)?;
            Ok(DiscoveryEvidenceRow {
                source: crate::discovery::DiscoverySource::parse(&source)
                    .map(|s| s.as_str().to_string())
                    .unwrap_or(source),
                kind: crate::discovery::EvidenceKind::parse(&kind)
                    .map(|k| k.as_str().to_string())
                    .unwrap_or(kind),
                key: r.get(2)?,
                value: r.get(3)?,
                confidence: crate::discovery::Confidence::parse(&confidence)
                    .as_str()
                    .to_string(),
                first_seen: r.get(5)?,
                last_seen: r.get(6)?,
                freshness: crate::discovery::freshness(misses).as_str().to_string(),
                misses,
            })
        })
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;

    // How current the freshest discovery claim on file is. The filter matches
    // the one in `write_discovery` and in INVENTORY_SQL: exactly the claims an
    // ordinary mDNS and SSDP pass re-hears, because a claim that cannot age
    // would otherwise hold the answer at "current" for ever.
    let best_misses: Option<i64> = conn
        .query_row(
            "SELECT MIN(misses) FROM discovery_evidence
             WHERE device_id = ?1 AND source IN ('mdns', 'ssdp')
               AND kind NOT IN ('manufacturer', 'model', 'model_number', 'serial_number', 'url')",
            params![device_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_err)?
        .flatten();
    let state = crate::discovery::freshness(best_misses.unwrap_or(0));
    record.evidence_freshness = state.as_str().to_string();

    // The classifier's own answer is kept, and the displayed one is reduced
    // where every claim behind it has gone stale. Reducing at read time rather
    // than at write time is deliberate: nothing is rewritten, no scan is
    // needed to undo it, and a device that starts answering again is back at
    // full confidence the moment its evidence is confirmed.
    record.raw_type_confidence = record.type_confidence.clone();
    record.type_confidence = crate::discovery::cap_for_freshness(
        crate::discovery::Confidence::parse(&record.type_confidence),
        state,
    )
    .as_str()
    .to_string();

    Ok(Some(record))
}

/// Rebuild a [`Classification`] from stored strings, so the reconcile rule that
/// protects a settled type is the same tested function the scanner used.
fn classification_from(
    device_type: &Option<String>,
    confidence: &Option<String>,
    evidence: &[String],
) -> crate::discovery::Classification {
    crate::discovery::Classification {
        device_type: crate::discovery::DeviceType::parse(
            device_type.as_deref().unwrap_or("unknown"),
        ),
        confidence: crate::discovery::Confidence::parse(confidence.as_deref().unwrap_or("unknown")),
        evidence: evidence.to_vec(),
        conflicts: Vec::new(),
    }
}

/// Which device, in which scan, a discovery comparison is about.
struct DiscoverySubject<'a> {
    scan_id: i64,
    scope_id: i64,
    baseline: &'a BaselineScan,
    device_id: i64,
    /// The device's name as this scan would show it, copied onto the event so
    /// the record stays readable after the device is gone.
    label: &'a str,
    ip: &'a str,
    now: &'a str,
    /// True when the operator marked the device Ignored, in which case its
    /// events are recorded already-ignored rather than dropped.
    ignored: bool,
}

/// Compare a device's discovery record before and after this scan, and write an
/// event for each change worth a person's attention.
///
/// # What is deliberately silent
///
/// * a name or type that only changed in whitespace, punctuation or case —
///   [`crate::discovery::model::normalize_value`] decides equality
/// * anything below high confidence, on either side of the comparison
/// * a service that went missing from fewer than [`SERVICE_ABSENCE_MISSES`]
///   consecutive full-discovery scans
/// * **evidence simply getting older.** Aging changes a miss counter and
///   nothing else. The type and confidence compared here are the classifier's
///   own answer from the stored record, which no miss counter touches, so a
///   device whose evidence went stale between two scans produces no event at
///   all. The reduction that stale evidence causes happens at *read* time, for
///   display, and is deliberately not part of what is compared.
/// * **an operator setting or clearing a device-type override.** That is an
///   edit to ArcScan, not an event on the network, and it is written by
///   [`Db::set_device_type_override`], which touches one column on `devices`
///   and never reaches this function.
/// * a name or model difference that is this release's tidier naming rules
///   rather than the device — see [`NAMING_RULES_VERSION`]
/// * every protocol housekeeping value there is: TTL, `CACHE-CONTROL`,
///   `BOOTID.UPNP.ORG`, `CONFIGID`, `SEARCHPORT`, the `SERVER` banner, TXT key
///   ordering, and repeated advertisements of something already known
/// * a device with no previous discovery record at all — the first time
///   ArcScan hears a device is not a change, it is the baseline
///
/// The last one matters most in practice: without it, the first scan after
/// upgrading to v1.8.2 would report a detected name and a device type for every
/// device on the network at once.
fn record_discovery_events(
    tx: &Transaction<'_>,
    subject: &DiscoverySubject<'_>,
    before: Option<&DiscoverySnapshot>,
    after: &DiscoverySnapshot,
) -> Result<usize, String> {
    let &DiscoverySubject {
        scan_id,
        scope_id,
        baseline,
        device_id,
        label,
        ip,
        now,
        ignored,
    } = subject;

    // Nothing to compare against: this device had no discovery record before.
    let Some(before) = before else {
        return Ok(0);
    };

    let mut events: Vec<(ChangeType, Option<String>, Option<String>)> = Vec::new();
    let changed = |a: &Option<String>, b: &Option<String>| -> bool {
        let norm = |v: &Option<String>| {
            v.as_deref()
                .map(crate::discovery::model::normalize_value)
                .filter(|s| !s.is_empty())
        };
        norm(a) != norm(b)
    };

    // The stored record predates this build's naming rules, so any difference
    // between the two sides is this release tidying a name rather than the
    // device changing one. Silent exactly once: the record written by this scan
    // carries the current generation, so the next scan compares normally.
    //
    // Without this, the first scan after upgrading to v1.8.3 would report a
    // renamed device for every device whose advertised name contained a
    // service-instance suffix, a repeated manufacturer or a UDN — a whole
    // inbox of events for nothing that happened on the network.
    let naming_rules_changed = before.naming_rules_version < NAMING_RULES_VERSION;

    // A detected name is only news when both readings were strong. A device
    // that fell back to its address this scan has not been renamed.
    if !naming_rules_changed
        && before.name_is_strong
        && after.name_is_strong
        && changed(&before.detected_name, &after.detected_name)
    {
        events.push((
            ChangeType::DetectedNameChanged,
            before.detected_name.clone(),
            after.detected_name.clone(),
        ));
    }

    // Type changes need high confidence on both sides. A device drifting
    // between Unknown and a low-confidence guess is not a change; it is the
    // classifier being honest about weak evidence.
    let strong = |c: &str| c == "high";
    if strong(&before.type_confidence)
        && strong(&after.type_confidence)
        && before.device_type != after.device_type
    {
        events.push((
            ChangeType::DeviceTypeChanged,
            Some(before.device_type.clone()),
            Some(after.device_type.clone()),
        ));
    }

    // The model line is built by `names::manufacturer_and_model`, which v1.8.3
    // also changed, so it is suppressed under the same rule and for the same
    // reason.
    if !naming_rules_changed
        && (changed(&before.manufacturer, &after.manufacturer)
            || changed(&before.model_name, &after.model_name))
    {
        let describe = |snapshot: &DiscoverySnapshot| {
            crate::discovery::names::manufacturer_and_model(
                snapshot.manufacturer.as_deref(),
                snapshot.model_name.as_deref(),
            )
        };
        // Only when both sides actually say something. Learning a model for the
        // first time is enrichment, not a change of hardware.
        let (from, to) = (describe(before), describe(after));
        if from.is_some() && to.is_some() && from != to {
            events.push((ChangeType::ModelChanged, from, to));
        }
    }

    let before_services: std::collections::BTreeSet<&String> = before.services.iter().collect();
    let after_services: std::collections::BTreeSet<&String> = after.services.iter().collect();
    for service in after_services.difference(&before_services) {
        events.push((ChangeType::ServiceAppeared, None, Some((*service).clone())));
    }
    for service in before_services.difference(&after_services) {
        events.push((
            ChangeType::ServiceDisappeared,
            Some((*service).clone()),
            None,
        ));
    }

    if events.is_empty() {
        return Ok(0);
    }

    let state = if ignored {
        ChangeState::Ignored
    } else {
        ChangeState::Unreviewed
    };
    let mut stmt = tx
        .prepare(
            "INSERT INTO change_events
                (event_key, scan_id, baseline_scan_id, network_scope_id, device_id, device_label,
                 ip, mac, vendor, change_type, old_value, new_value, details, state, created_at,
                 scan_at, baseline_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?10, NULL, ?11, ?12, ?12, ?13)
             ON CONFLICT(event_key) DO NOTHING",
        )
        .map_err(sql_err)?;

    let mut written = 0usize;
    for (kind, old, new) in events {
        // The key includes the value so two services appearing in one scan are
        // two events rather than one overwriting the other, and so a retried
        // save still cannot duplicate either.
        let discriminator = new.clone().or_else(|| old.clone()).unwrap_or_default();
        let key = format!(
            "s{scan_id}|d{device_id}|{}|{}",
            kind.as_str(),
            crate::discovery::model::normalize_value(&discriminator)
        );
        written += stmt
            .execute(params![
                key,
                scan_id,
                baseline.id,
                scope_id,
                device_id,
                label,
                ip,
                kind.as_str(),
                old,
                new,
                state.as_str(),
                now,
                baseline.created_at,
            ])
            .map_err(sql_err)?;
    }
    Ok(written)
}

/// Turn a rusqlite error into a message safe to show a person. The raw SQL error
/// is kept in the text because it is the only clue when a database is corrupt or
/// read-only, and the UI shows it under an expandable technical section rather
/// than as the headline.
fn sql_err(e: rusqlite::Error) -> String {
    format!("Scan database error: {e}")
}

/// A scan's identifying fields, used as a comparison baseline.
struct BaselineScan {
    id: i64,
    target: String,
    created_at: String,
    /// What discovery the baseline managed. Discovery-derived comparisons only
    /// run when this and the new scan are both `full`.
    discovery_mode: String,
}

/// The most recent *completed* scan in the same network scope that covers the
/// same normalized target with the same coverage. `before` excludes the scan
/// being compared and everything after it.
///
/// `status = 'completed'` is the partial-scan safety rule: a cancelled scan
/// never becomes a baseline — for completed scans, for other cancelled scans,
/// for history comparison or for change notifications — because absence from a
/// scan that did not check every address proves nothing.
fn find_baseline(
    tx: &Transaction<'_>,
    scope_id: Option<i64>,
    target_key: &str,
    coverage_key: &str,
    before: Option<i64>,
) -> Result<Option<BaselineScan>, String> {
    // `discovery_mode` is selected but deliberately not filtered on: a scan
    // that heard no multicast is still a perfectly good baseline for hosts and
    // ports, which is what a baseline is mainly for. Only the discovery-derived
    // comparison consults the mode, and it does so separately.
    let sql = "SELECT id, target, created_at, discovery_mode FROM scans
               WHERE network_scope_id IS ?1
                 AND target_key = ?2
                 AND coverage_key = ?3
                 AND status = 'completed'
                 AND id < ?4
               ORDER BY id DESC LIMIT 1";
    tx.query_row(
        sql,
        params![
            scope_id,
            target_key,
            coverage_key,
            before.unwrap_or(i64::MAX)
        ],
        |row| {
            Ok(BaselineScan {
                id: row.get(0)?,
                target: row.get(1)?,
                created_at: row.get(2)?,
                discovery_mode: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(sql_err)
}

/// Resolve which network scope a scan belongs to, creating one when needed.
///
/// The scope's anchor is the canonical network: for a scan that ran against one
/// of this machine's own subnets, the subnet itself (so a single-host scan and
/// a full sweep of the same LAN share a scope); for a routed scan, the
/// canonical target. The default gateway's MAC, when the scanner could observe
/// it, disambiguates unrelated networks that reuse the same private range:
///
/// * A scope whose recorded gateway matches is reused.
/// * A scope with no recorded gateway adopts the newly observed one — it was
///   created before the gateway was learnable (or by migration).
/// * A different recorded gateway means a genuinely different network behind
///   the same addressing, so a new scope is created.
///
/// Without gateway evidence the most recently used scope for the network is
/// reused, preferring continuity over inventing scopes — creation must stay
/// usable when gateway information is unavailable.
fn resolve_scope(
    tx: &Transaction<'_>,
    result: &ScanResult,
    target_key: &str,
    now: &str,
) -> Result<i64, String> {
    let hint = result.scope_hint.as_ref();
    let (scope_target, default_name) = match hint.and_then(|h| h.local_network.as_deref()) {
        Some(cidr) => (
            ipparse::canonical_key(cidr).unwrap_or_else(|_| format!("cidr:{cidr}")),
            cidr.to_string(),
        ),
        None => (target_key.to_string(), result.target.clone()),
    };
    let gateway_mac = hint
        .and_then(|h| h.gateway_mac.as_deref())
        .and_then(inventory::normalize_mac);
    let interface = hint.and_then(|h| h.interface.clone());

    // Most recently used first, with the id breaking ties, so which scope
    // adopts a newly learned gateway — or gets reused when there is no gateway
    // evidence at all — is both deterministic and the one still in active use.
    let existing: Vec<(i64, Option<String>)> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, gateway_mac FROM network_scopes
                 WHERE canonical_target = ?1
                 ORDER BY updated_at DESC, id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![scope_target], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };

    let touch = |id: i64, learned_mac: Option<&str>| -> Result<i64, String> {
        tx.execute(
            "UPDATE network_scopes
             SET updated_at = ?1,
                 gateway_mac = COALESCE(?2, gateway_mac),
                 interface_hint = COALESCE(?3, interface_hint)
             WHERE id = ?4",
            params![now, learned_mac, interface, id],
        )
        .map_err(sql_err)?;
        Ok(id)
    };

    let reused = match &gateway_mac {
        Some(gw) => {
            if let Some((id, _)) = existing.iter().find(|(_, mac)| mac.as_deref() == Some(gw)) {
                Some(touch(*id, None)?)
            } else if let Some((id, _)) = existing.iter().find(|(_, mac)| mac.is_none()) {
                // Adopt: the scope predates gateway evidence for this network.
                Some(touch(*id, Some(gw))?)
            } else {
                None // every known scope has a *different* gateway
            }
        }
        None => match existing.first() {
            Some((id, _)) => Some(touch(*id, None)?),
            None => None,
        },
    };
    if let Some(id) = reused {
        return Ok(id);
    }

    let stable_key = match &gateway_mac {
        Some(gw) => format!("target:{scope_target}|gw:{gw}"),
        None => format!("target:{scope_target}"),
    };
    tx.execute(
        "INSERT INTO network_scopes
            (stable_key, display_name, canonical_target, gateway_mac, interface_hint,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(stable_key) DO UPDATE SET updated_at = excluded.updated_at",
        params![
            stable_key,
            default_name,
            scope_target,
            gateway_mac,
            interface,
            now
        ],
    )
    .map_err(sql_err)?;
    tx.query_row(
        "SELECT id FROM network_scopes WHERE stable_key = ?1",
        params![stable_key],
        |r| r.get(0),
    )
    .map_err(sql_err)
}

/// Load one scan's observations already paired with their device identities.
fn load_identified(tx: &Transaction<'_>, scan_id: i64) -> Result<Vec<IdentifiedHost>, String> {
    let mut stmt = tx
        .prepare(
            "SELECT h.ip, h.hostname, h.mac, h.vendor, h.open_ports, h.response_ms,
                    h.icmp_ms, h.tcp_ms, h.ttl, h.os_guess, h.last_seen,
                    h.device_id, d.identity_key, d.custom_name, d.first_seen
             FROM hosts h LEFT JOIN devices d ON d.id = h.device_id
             WHERE h.scan_id = ?1
             ORDER BY h.id ASC",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![scan_id], |row| {
            let host = read_host(row)?;
            let device_id: Option<i64> = row.get(11)?;
            let stored_key: Option<String> = row.get(12)?;
            Ok(IdentifiedHost {
                device_id,
                // Rows written before the inventory existed have no device link
                // until the backfill runs; deriving the key keeps them
                // comparable either way.
                identity_key: stored_key.unwrap_or_else(|| inventory::identify(&host).key),
                custom_name: row.get(13)?,
                previously_known: false,
                host,
            })
        })
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    Ok(rows)
}

/// The device a host resolved to, plus whether it was already in the inventory.
struct DeviceRecord {
    id: i64,
    identity_key: String,
    custom_name: Option<String>,
    /// False when this scan created the device, which is what makes a device
    /// genuinely new rather than merely absent from the baseline scan.
    existed: bool,
}

/// Find or create the device for one observation, *within one network scope*.
///
/// Every lookup below is bounded by `network_scope_id`: the same MAC, hostname
/// or address on a different scope is a different device, so names, notes,
/// status and history can never leak between two client networks.
fn upsert_device(
    tx: &Transaction<'_>,
    scope_id: i64,
    host: &HostResult,
    seen_at: &str,
) -> Result<DeviceRecord, String> {
    let identity = inventory::identify(host);

    // Look up by MAC first: a device seen earlier without one (routed scan, or a
    // scan where ARP had not resolved yet) was stored under a hostname or IP key,
    // and must be recognised rather than duplicated.
    let existing = if let Some(mac) = &identity.mac {
        tx.query_row(
            "SELECT id, identity_key, custom_name FROM devices
             WHERE network_scope_id = ?1 AND mac = ?2",
            params![scope_id, mac],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sql_err)?
    } else {
        None
    };
    let existing = match existing {
        Some(found) => Some(found),
        None => tx
            .query_row(
                "SELECT id, identity_key, custom_name FROM devices
                 WHERE network_scope_id = ?1 AND identity_key = ?2",
                params![scope_id, identity.key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?,
    };

    // A MAC-identified observation can also claim a MAC-less device in the same
    // scope that matches on hostname and vendor, or on the same address. This is
    // the common case of ARP resolving on a later scan, and adopting the old row
    // keeps the device's first-seen date, name and notes.
    let existing = match (existing, &identity.mac) {
        (None, Some(_)) => {
            let fallback_key = {
                let mut probe = host.clone();
                probe.mac = None;
                inventory::identify(&probe).key
            };
            tx.query_row(
                "SELECT id, identity_key, custom_name FROM devices
                 WHERE network_scope_id = ?1 AND mac IS NULL
                   AND (identity_key = ?2 OR identity_key = ?3)
                 ORDER BY id ASC LIMIT 1",
                params![scope_id, fallback_key, format!("ip:{}", host.ip)],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?
        }
        (existing, _) => existing,
    };

    if let Some((id, stored_key, custom_name)) = existing {
        // Only overwrite with information we actually have: a scan that could
        // not resolve a hostname must not erase the name a previous scan found.
        tx.execute(
            "UPDATE devices SET
                 identity_key    = ?1,
                 identity_source = ?2,
                 mac             = COALESCE(?3, mac),
                 hostname        = COALESCE(?4, hostname),
                 vendor          = COALESCE(?5, vendor),
                 last_ip         = ?6,
                 last_seen       = ?7
             WHERE id = ?8",
            params![
                identity.key,
                source_str(identity.source),
                identity.mac,
                host.hostname,
                host.vendor,
                host.ip,
                seen_at,
                id,
            ],
        )
        .map_err(sql_err)?;
        let _ = stored_key;
        return Ok(DeviceRecord {
            id,
            identity_key: identity.key,
            custom_name,
            existed: true,
        });
    }

    tx.execute(
        "INSERT INTO devices
            (network_scope_id, identity_key, identity_source, mac, hostname, vendor, last_ip,
             first_seen, last_seen, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'unclassified')",
        params![
            scope_id,
            identity.key,
            source_str(identity.source),
            identity.mac,
            host.hostname,
            host.vendor,
            host.ip,
            seen_at,
        ],
    )
    .map_err(sql_err)?;
    Ok(DeviceRecord {
        id: tx.last_insert_rowid(),
        identity_key: identity.key,
        custom_name: None,
        existed: false,
    })
}

fn insert_observation(
    tx: &Transaction<'_>,
    scan_id: i64,
    device_id: i64,
    host: &HostResult,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO hosts
            (scan_id, device_id, ip, hostname, mac, vendor, open_ports, response_ms,
             icmp_ms, tcp_ms, ttl, os_guess, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            scan_id,
            device_id,
            host.ip,
            host.hostname,
            host.mac,
            host.vendor,
            format_ports(&host.open_ports),
            host.response_ms.map(|v| v as i64),
            host.icmp_ms,
            host.tcp_ms,
            host.ttl.map(|v| v as i64),
            host.os_guess,
            host.last_seen,
        ],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn source_str(source: IdentitySource) -> &'static str {
    match source {
        IdentitySource::Mac => "mac",
        IdentitySource::HostnameVendor => "hostname-vendor",
        IdentitySource::Ip => "ip",
    }
}

fn parse_source(s: &str) -> IdentitySource {
    match s {
        "mac" => IdentitySource::Mac,
        "hostname-vendor" => IdentitySource::HostnameVendor,
        _ => IdentitySource::Ip,
    }
}

fn format_ports(ports: &[u16]) -> String {
    ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_ports(stored: &str) -> Vec<u16> {
    stored
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.trim().parse::<u16>().ok())
        .collect()
}

/// Read the 11 host columns shared by every observation query, in order.
fn read_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostResult> {
    Ok(HostResult {
        ip: row.get(0)?,
        hostname: row.get(1)?,
        mac: row.get(2)?,
        vendor: row.get(3)?,
        open_ports: parse_ports(&row.get::<_, String>(4)?),
        response_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        icmp_ms: row.get(6)?,
        tcp_ms: row.get(7)?,
        ttl: row.get::<_, Option<i64>>(8)?.map(|v| v as u8),
        os_guess: row.get(9)?,
        last_seen: row.get(10)?,
        // Discovery is stored per device rather than per observation, so a
        // replayed observation carries none. The drawer reads the device's
        // current discovery record instead.
        discovery: None,
    })
}

fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanSummary> {
    // Parsed once here rather than per field below, and rendered into the two
    // words History shows. A scan recorded before discovery existed has no
    // summary at all, and "skipped" is the honest reading of that.
    let discovery_summary: Option<String> = row.get(18)?;
    let report = discovery_summary
        .as_deref()
        .and_then(|raw| serde_json::from_str::<crate::discovery::DiscoveryReport>(raw).ok());
    let quality = report
        .as_ref()
        .map(|r| r.quality())
        .unwrap_or(crate::discovery::DiscoveryQuality::Skipped);
    let quality_reason = report.as_ref().and_then(|r| r.quality_reason());

    Ok(ScanSummary {
        id: row.get(0)?,
        target: row.get(1)?,
        target_key: row.get(2)?,
        profile: row.get(3)?,
        created_at: row.get(4)?,
        duration_ms: row.get(5)?,
        scanned: row.get(6)?,
        probed: row.get(7)?,
        host_count: row.get(8)?,
        new_count: row.get(9)?,
        missing_count: row.get(10)?,
        changed_count: row.get(11)?,
        status: row.get(12)?,
        baseline_scan_id: row.get(13)?,
        network_scope_id: row.get(14)?,
        scope_name: row.get(15)?,
        coverage_key: row.get(16)?,
        // Normalized through the vocabulary so a value from a newer build, or a
        // row an older build left blank, reads as a mode the interface knows.
        discovery_mode: crate::discovery::model::DiscoveryMode::parse(&row.get::<_, String>(17)?)
            .as_str()
            .to_string(),
        discovery_summary,
        discovery_quality: quality.as_str().to_string(),
        discovery_quality_reason: quality_reason.map(str::to_string),
    })
}

fn read_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        network_scope_id: row.get(1)?,
        identity_key: row.get(2)?,
        identity_source: parse_source(&row.get::<_, String>(3)?),
        mac: row.get(4)?,
        custom_name: row.get(5)?,
        hostname: row.get(6)?,
        vendor: row.get(7)?,
        last_ip: row.get(8)?,
        first_seen: row.get(9)?,
        last_seen: row.get(10)?,
        status: DeviceStatus::parse(&row.get::<_, String>(11)?),
        notes: row.get(12)?,
        observation_count: row.get(13)?,
        // Normalized through the type vocabulary, so a value from a newer build
        // reads as absent rather than as an unknown word the interface has no
        // label for. An unrecognised override is not a silent Unknown: Unknown
        // is itself a deliberate answer, and inventing one would be worse.
        user_device_type: row.get::<_, Option<String>>(14)?.and_then(|raw| {
            crate::discovery::DeviceType::parse_strict(&raw).map(|t| t.as_str().to_string())
        }),
    })
}

/// Present a stored observation as a `HostResult` so the shared diff logic can
/// be reused for the device drawer's "recent changes".
fn observation_as_host(obs: &DeviceObservation) -> HostResult {
    HostResult {
        ip: obs.ip.clone(),
        hostname: obs.hostname.clone(),
        mac: None,
        vendor: obs.vendor.clone(),
        open_ports: obs.open_ports.clone(),
        response_ms: obs.response_ms,
        icmp_ms: obs.icmp_ms,
        tcp_ms: obs.tcp_ms,
        ttl: obs.ttl,
        os_guess: obs.os_guess.clone(),
        last_seen: obs.observed_at.clone(),
        discovery: None,
    }
}

/// Create or upgrade the schema. Safe to run on every launch.
fn migrate(conn: &mut Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS schema_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS scans (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            target      TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL,
            scanned     INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hosts (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            scan_id     INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            ip          TEXT NOT NULL,
            hostname    TEXT,
            mac         TEXT,
            vendor      TEXT,
            open_ports  TEXT NOT NULL,
            response_ms INTEGER,
            ttl         INTEGER,
            os_guess    TEXT,
            last_seen   TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_hosts_scan ON hosts(scan_id);

        CREATE TABLE IF NOT EXISTS network_scopes (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            stable_key       TEXT NOT NULL UNIQUE,
            display_name     TEXT NOT NULL,
            canonical_target TEXT,
            gateway_mac      TEXT,
            interface_hint   TEXT,
            created_at       TEXT NOT NULL,
            updated_at       TEXT NOT NULL
        );
        "#,
    )
    .map_err(sql_err)?;

    // Column additions, oldest first. SQLite has no "ADD COLUMN IF NOT EXISTS",
    // so a duplicate-column error means the migration already ran.
    for stmt in [
        // v1.6.x additions, kept so a database from before them still upgrades.
        "ALTER TABLE hosts ADD COLUMN ttl INTEGER",
        "ALTER TABLE hosts ADD COLUMN os_guess TEXT",
        // v1.7: split latency measurements and the device link.
        "ALTER TABLE hosts ADD COLUMN icmp_ms REAL",
        "ALTER TABLE hosts ADD COLUMN tcp_ms REAL",
        "ALTER TABLE hosts ADD COLUMN device_id INTEGER REFERENCES devices(id) ON DELETE SET NULL",
        // v1.7: scan metadata for comparison and the history list.
        "ALTER TABLE scans ADD COLUMN target_key TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE scans ADD COLUMN profile TEXT",
        "ALTER TABLE scans ADD COLUMN probed INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE scans ADD COLUMN status TEXT NOT NULL DEFAULT 'completed'",
        "ALTER TABLE scans ADD COLUMN new_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE scans ADD COLUMN missing_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE scans ADD COLUMN changed_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE scans ADD COLUMN baseline_scan_id INTEGER",
        // v1.7.1: network scope and comparison signature.
        "ALTER TABLE scans ADD COLUMN network_scope_id INTEGER REFERENCES network_scopes(id)",
        "ALTER TABLE scans ADD COLUMN coverage_key TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE scans ADD COLUMN execution_settings TEXT",
        // v1.8.2: what the scan's discovery pass managed to do.
        //
        // `discovery_mode` is deliberately *not* part of `coverage_key`. Port
        // and presence comparison must stay exactly as compatible as it was:
        // two scans of the same ports still compare, whether or not either of
        // them heard a multicast response. Discovery-derived comparisons
        // consult this column separately.
        "ALTER TABLE scans ADD COLUMN discovery_mode TEXT NOT NULL DEFAULT 'none'",
        "ALTER TABLE scans ADD COLUMN discovery_summary TEXT",
    ] {
        let _ = conn.execute(stmt, []);
    }

    // A fresh database gets the scoped (v3) devices shape immediately; an
    // existing pre-v3 table keeps its shape here and is rebuilt by migrate_v3.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS devices (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            network_scope_id INTEGER NOT NULL REFERENCES network_scopes(id),
            identity_key     TEXT NOT NULL,
            identity_source  TEXT NOT NULL,
            mac              TEXT,
            custom_name      TEXT,
            hostname         TEXT,
            vendor           TEXT,
            last_ip          TEXT,
            first_seen       TEXT NOT NULL,
            last_seen        TEXT NOT NULL,
            status           TEXT NOT NULL DEFAULT 'unclassified',
            notes            TEXT,
            -- v1.8.3: the operator's device-type correction. NULL is Auto;
            -- 'unknown' is an explicit answer and not the absence of one.
            user_device_type TEXT,
            UNIQUE(network_scope_id, identity_key),
            UNIQUE(network_scope_id, mac)
        );
        "#,
    )
    .map_err(sql_err)?;

    let version: i64 = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_err)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    // v1.8: normalized change events. Created before migrate_v4 so a fresh
    // database and an upgraded one reach exactly the same shape.
    //
    // `scan_id` and `baseline_scan_id` are deliberately *not* foreign keys onto
    // `scans`: retention prunes old scans, and a change that was reviewed months
    // ago should stay readable afterwards. The scan timestamps and the device
    // label are copied in for the same reason.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS change_events (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            event_key        TEXT NOT NULL UNIQUE,
            scan_id          INTEGER,
            baseline_scan_id INTEGER,
            network_scope_id INTEGER,
            device_id        INTEGER REFERENCES devices(id) ON DELETE CASCADE,
            device_label     TEXT NOT NULL,
            ip               TEXT,
            mac              TEXT,
            vendor           TEXT,
            change_type      TEXT NOT NULL,
            old_value        TEXT,
            new_value        TEXT,
            details          TEXT,
            state            TEXT NOT NULL DEFAULT 'unreviewed',
            created_at       TEXT NOT NULL,
            scan_at          TEXT,
            baseline_at      TEXT,
            acknowledged_at  TEXT
        );
        "#,
    )
    .map_err(sql_err)?;

    // v1.8.2: local discovery. Two tables, created for every database so a
    // fresh install and an upgraded one reach the same shape.
    //
    // `discovery_evidence` is the durable record: one row per distinct claim,
    // per source, per device. The unique index is what makes a re-observation
    // an update rather than a new row, so a device advertising the same service
    // every day for a year costs one row, not 365.
    //
    // `device_discovery` is the resolved answer — the name, type and confidence
    // the evidence adds up to. Derived, rewritten whenever the evidence
    // changes, and never the source of truth: deleting it and recomputing from
    // `discovery_evidence` would produce exactly the same thing. It exists so
    // the Inventory query stays two statements rather than one per row.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS discovery_evidence (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id        INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
            network_scope_id INTEGER,
            source           TEXT NOT NULL,
            kind             TEXT NOT NULL,
            key              TEXT NOT NULL DEFAULT '',
            value            TEXT NOT NULL,
            normalized_value TEXT NOT NULL,
            confidence       TEXT NOT NULL DEFAULT 'unknown',
            first_seen       TEXT NOT NULL,
            last_seen        TEXT NOT NULL,
            last_scan_id     INTEGER,
            -- Consecutive full-discovery scans that did not re-observe this
            -- claim. Reset to 0 on every sighting; see SERVICE_ABSENCE_MISSES.
            misses           INTEGER NOT NULL DEFAULT 0,
            metadata_json    TEXT,
            UNIQUE(device_id, source, kind, key, normalized_value)
        );

        CREATE TABLE IF NOT EXISTS device_discovery (
            device_id          INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
            network_scope_id   INTEGER,
            detected_name      TEXT,
            name_source        TEXT,
            device_type        TEXT NOT NULL DEFAULT 'unknown',
            type_confidence    TEXT NOT NULL DEFAULT 'unknown',
            type_evidence      TEXT,
            type_conflicts     TEXT,
            manufacturer       TEXT,
            model_name         TEXT,
            model_number       TEXT,
            serial_number      TEXT,
            mdns_hostname      TEXT,
            ssdp_friendly_name TEXT,
            services           TEXT,
            sources            TEXT,
            alternate_names    TEXT,
            ipv6_addresses     TEXT,
            presentation_url   TEXT,
            first_discovered_at TEXT,
            last_discovered_at TEXT,
            -- v1.8.3: which generation of the naming rules wrote the values
            -- above. See NAMING_RULES_VERSION.
            naming_rules_version INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(sql_err)?;

    if version < 2 {
        backfill_v2(conn)?;
    }
    if version < 3 {
        migrate_v3(conn)?;
    }
    if version < 4 {
        migrate_v4(conn)?;
    }
    if version < 5 {
        migrate_v5(conn)?;
    }

    // v1.8.3 (schema 6). Two nullable columns and nothing else: no table is
    // rebuilt, no row is re-keyed, no device is reclassified and nothing is
    // backfilled. An interrupted upgrade leaves a database that is either
    // before or after, and running it twice changes nothing the second time.
    //
    // Deliberately *after* migrate_v3, which rebuilds the devices table to give
    // it network scopes: a column added before that rebuild would be dropped by
    // it, and a pre-v1.8.2 database has no `device_discovery` table to alter at
    // all until the statements above have created it.
    //
    // `devices.user_device_type` is the operator's correction. NULL means Auto;
    // the string `unknown` means the operator looked and said so, which is a
    // different and equally valid answer. It lives on `devices` rather than on
    // `device_discovery` on purpose: a device no discovery-capable scan has
    // ever reached has no `device_discovery` row, and must still be
    // correctable.
    //
    // `device_discovery.naming_rules_version` defaults to 0, which is exactly
    // right for every row v1.8.2 wrote: older than the current rules, so the
    // first comparison against it is silent. See [`NAMING_RULES_VERSION`].
    //
    // Both are `ALTER TABLE ... ADD COLUMN`, which SQLite has no
    // `IF NOT EXISTS` form of, so a duplicate-column error means this migration
    // has already run and is ignored — the same idempotence rule every earlier
    // migration here uses.
    for stmt in [
        "ALTER TABLE devices ADD COLUMN user_device_type TEXT",
        "ALTER TABLE device_discovery ADD COLUMN naming_rules_version INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = conn.execute(stmt, []);
    }

    // Indexes last: the scope-aware ones only exist once the v3 shape does.
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen DESC);
        CREATE INDEX IF NOT EXISTS idx_devices_scope_mac ON devices(network_scope_id, mac);
        CREATE INDEX IF NOT EXISTS idx_devices_scope     ON devices(network_scope_id);
        CREATE INDEX IF NOT EXISTS idx_devices_status    ON devices(status);
        CREATE INDEX IF NOT EXISTS idx_hosts_device      ON hosts(device_id);
        CREATE INDEX IF NOT EXISTS idx_hosts_device_scan ON hosts(device_id, scan_id DESC);
        CREATE INDEX IF NOT EXISTS idx_hosts_scan_ip     ON hosts(scan_id, ip);
        CREATE INDEX IF NOT EXISTS idx_scans_target_key  ON scans(target_key, id DESC);
        CREATE INDEX IF NOT EXISTS idx_scans_baseline
            ON scans(network_scope_id, target_key, coverage_key, status, id DESC);
        CREATE INDEX IF NOT EXISTS idx_scans_scope_completed
            ON scans(network_scope_id, status, id DESC);
        CREATE INDEX IF NOT EXISTS idx_change_events_created ON change_events(id DESC);
        CREATE INDEX IF NOT EXISTS idx_change_events_state   ON change_events(state, id DESC);
        CREATE INDEX IF NOT EXISTS idx_change_events_device  ON change_events(device_id, id DESC);
        CREATE INDEX IF NOT EXISTS idx_change_events_scope   ON change_events(network_scope_id, id DESC);
        CREATE INDEX IF NOT EXISTS idx_discovery_evidence_device
            ON discovery_evidence(device_id, kind);
        CREATE INDEX IF NOT EXISTS idx_discovery_evidence_scope
            ON discovery_evidence(network_scope_id);
        CREATE INDEX IF NOT EXISTS idx_device_discovery_type
            ON device_discovery(device_type);
        "#,
    )
    .map_err(sql_err)?;

    conn.execute(
        "INSERT INTO schema_meta (key, value) VALUES ('version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION.to_string()],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Backfill the v1.7 scan columns from existing v1.6 rows: normalize every
/// scan's target into a comparison key and mark completed coverage. Building
/// the device inventory from old observations happens in [`migrate_v3`], which
/// runs immediately afterwards and knows about network scopes.
fn backfill_v2(conn: &mut Connection) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_err)?;

    // Scan targets -> normalized keys. Only rows the earlier schema left blank.
    let blank: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, target FROM scans WHERE target_key = '' OR target_key IS NULL")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };
    for (id, target) in blank {
        let key = ipparse::canonical_key(&target).unwrap_or_else(|_| target.clone());
        tx.execute(
            "UPDATE scans SET target_key = ?1 WHERE id = ?2",
            params![key, id],
        )
        .map_err(sql_err)?;
    }
    // Historical scans predate the probed counter; they ran to completion, so
    // every address they enumerated was probed.
    tx.execute("UPDATE scans SET probed = scanned WHERE probed = 0", [])
        .map_err(sql_err)?;

    tx.commit().map_err(sql_err)
}

/// True when `table` has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sql_err)?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    Ok(names.iter().any(|n| n == column))
}

/// The v1.7.1 upgrade: network scopes and comparison signatures.
///
/// * Creates one scope per distinct historical target, so existing history
///   lands somewhere deterministic. Scope refinement (gateway MACs) happens as
///   new scans arrive; see [`resolve_scope`].
/// * Backfills every scan's coverage key from its stored profile. Legacy
///   custom scans whose port set was never persisted get a key unique to the
///   scan — they compare with nothing rather than comparing wrongly.
/// * Rebuilds the devices table with per-scope composite uniqueness, keeping
///   ids, names, notes, status and first/last-seen intact. A device keeps all
///   its observations; it is assigned to the scope of its most recent one.
///   Devices with no remaining observations go to a `legacy` scope rather than
///   being guessed into a network they may not belong to.
/// * Links any still-unlinked observations (a v1.6 database) to scoped devices,
///   oldest scan first so first-seen dates stay truthful.
///
/// Everything runs in one transaction (foreign keys are re-enabled afterwards
/// either way), so an interrupted upgrade leaves the database exactly as it
/// was. Re-running is a no-op: every step checks for work left to do.
fn migrate_v3(conn: &mut Connection) -> Result<(), String> {
    // The devices rebuild recreates a table other tables reference, which
    // SQLite only allows with foreign-key enforcement off. Restore it whether
    // or not the migration succeeds.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(sql_err)?;
    let outcome = migrate_v3_inner(conn);
    let restore = conn.execute_batch("PRAGMA foreign_keys = ON;");
    outcome?;
    restore.map_err(sql_err)
}

fn migrate_v3_inner(conn: &mut Connection) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();
    let needs_rebuild = !column_exists(conn, "devices", "network_scope_id")?;
    let tx = conn.transaction().map_err(sql_err)?;

    // 1. One scope per distinct historical target key. The display name is the
    //    most recent raw target string, which is what the operator recognises.
    let targets: Vec<(String, String)> = {
        // SQLite's documented bare-column-with-MAX behaviour: `target` comes
        // from the row holding MAX(id), i.e. the most recent scan of the key.
        let mut stmt = tx
            .prepare(
                "SELECT target_key, target, MAX(id) FROM scans
                 WHERE network_scope_id IS NULL
                 GROUP BY target_key",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };
    for (key, display) in &targets {
        tx.execute(
            "INSERT INTO network_scopes
                (stable_key, display_name, canonical_target, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(stable_key) DO NOTHING",
            params![format!("target:{key}"), display, key, now],
        )
        .map_err(sql_err)?;
    }
    tx.execute(
        "UPDATE scans SET network_scope_id =
            (SELECT ns.id FROM network_scopes ns
             WHERE ns.stable_key = 'target:' || scans.target_key)
         WHERE network_scope_id IS NULL",
        [],
    )
    .map_err(sql_err)?;

    // 2. Coverage keys for scans saved before they existed.
    let uncovered: Vec<(i64, Option<String>)> = {
        let mut stmt = tx
            .prepare("SELECT id, profile FROM scans WHERE coverage_key = ''")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };
    for (id, profile) in uncovered {
        tx.execute(
            "UPDATE scans SET coverage_key = ?1 WHERE id = ?2",
            params![signature::legacy_coverage_key(profile.as_deref(), id), id],
        )
        .map_err(sql_err)?;
    }

    // 3. Rebuild the devices table into the scoped shape, preserving ids.
    if needs_rebuild {
        let orphans: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM devices d
                 WHERE NOT EXISTS (SELECT 1 FROM hosts h WHERE h.device_id = d.id)",
                [],
                |r| r.get(0),
            )
            .map_err(sql_err)?;
        if orphans > 0 {
            // A device whose scans were all pruned still carries the operator's
            // name and notes; keep it in a clearly-labelled scope instead of
            // guessing which network it belonged to.
            tx.execute(
                "INSERT INTO network_scopes
                    (stable_key, display_name, created_at, updated_at)
                 VALUES ('legacy', 'Earlier inventory', ?1, ?1)
                 ON CONFLICT(stable_key) DO NOTHING",
                params![now],
            )
            .map_err(sql_err)?;
        }

        tx.execute_batch(
            r#"
            CREATE TABLE devices_v3 (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                network_scope_id INTEGER NOT NULL REFERENCES network_scopes(id),
                identity_key     TEXT NOT NULL,
                identity_source  TEXT NOT NULL,
                mac              TEXT,
                custom_name      TEXT,
                hostname         TEXT,
                vendor           TEXT,
                last_ip          TEXT,
                first_seen       TEXT NOT NULL,
                last_seen        TEXT NOT NULL,
                status           TEXT NOT NULL DEFAULT 'unclassified',
                notes            TEXT,
                UNIQUE(network_scope_id, identity_key),
                UNIQUE(network_scope_id, mac)
            );

            INSERT INTO devices_v3
                (id, network_scope_id, identity_key, identity_source, mac, custom_name,
                 hostname, vendor, last_ip, first_seen, last_seen, status, notes)
            SELECT d.id,
                   COALESCE(
                       (SELECT s.network_scope_id
                        FROM hosts h JOIN scans s ON s.id = h.scan_id
                        WHERE h.device_id = d.id
                        ORDER BY h.scan_id DESC LIMIT 1),
                       (SELECT id FROM network_scopes WHERE stable_key = 'legacy')
                   ),
                   d.identity_key, d.identity_source, d.mac, d.custom_name,
                   d.hostname, d.vendor, d.last_ip, d.first_seen, d.last_seen,
                   d.status, d.notes
            FROM devices d;

            DROP TABLE devices;
            ALTER TABLE devices_v3 RENAME TO devices;
            "#,
        )
        .map_err(sql_err)?;
    }

    // 4. Observations that never got a device (a v1.6 database), oldest scan
    //    first so first_seen is truthful. Each scan's scope is known by now.
    let unlinked: Vec<(i64, i64, HostResult, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT h.id, s.network_scope_id, h.ip, h.hostname, h.mac, h.vendor,
                        h.open_ports, h.response_ms, h.icmp_ms, h.tcp_ms, h.ttl, h.os_guess,
                        h.last_seen, s.created_at
                 FROM hosts h JOIN scans s ON s.id = h.scan_id
                 WHERE h.device_id IS NULL
                 ORDER BY h.scan_id ASC, h.id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                let host_id: i64 = row.get(0)?;
                let scope_id: i64 = row.get(1)?;
                let host = HostResult {
                    ip: row.get(2)?,
                    hostname: row.get(3)?,
                    mac: row.get(4)?,
                    vendor: row.get(5)?,
                    open_ports: parse_ports(&row.get::<_, String>(6)?),
                    response_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                    icmp_ms: row.get(8)?,
                    tcp_ms: row.get(9)?,
                    ttl: row.get::<_, Option<i64>>(10)?.map(|v| v as u8),
                    os_guess: row.get(11)?,
                    last_seen: row.get(12)?,
                    discovery: None,
                };
                let scan_created: String = row.get(13)?;
                Ok((host_id, scope_id, host, scan_created))
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };
    for (host_id, scope_id, host, seen_at) in unlinked {
        let record = upsert_device(&tx, scope_id, &host, &seen_at)?;
        tx.execute(
            "UPDATE hosts SET device_id = ?1 WHERE id = ?2",
            params![record.id, host_id],
        )
        .map_err(sql_err)?;
    }

    tx.commit().map_err(sql_err)
}

/// The v1.8.0 upgrade: the Changes inbox starts empty, deliberately.
///
/// The `change_events` table is created for every database (see [`migrate`]).
/// What this migration decides is what goes *into* it for an existing install.
///
/// # Why nothing is backfilled
///
/// Two options were available: replay every historical comparison and store the
/// results as already-acknowledged, or start recording from the first scan after
/// the upgrade. Backfilling was rejected on both counts that matter here. It is
/// unbounded work at launch — a database with years of scans would replay every
/// one of them while the operator waits — and it would fill a brand-new feature
/// with entries nobody asked to review, which is exactly the backlog the release
/// is meant to avoid. Neither is worth it for changes that were already seen in
/// the comparison view at the time.
///
/// So the watermark below records the newest scan present at upgrade time. Every
/// scan saved afterwards records its changes normally, and the inbox explains
/// that it starts from the upgrade rather than looking silently empty. Scans
/// recorded before the upgrade keep their full comparison view, which is where
/// their history has always been.
///
/// Idempotent: the watermark is written once and never moved, so re-running the
/// migration (or opening an already-current database) changes nothing.
fn migrate_v4(conn: &mut Connection) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_err)?;
    let newest: i64 = tx
        .query_row("SELECT COALESCE(MAX(id), 0) FROM scans", [], |r| r.get(0))
        .map_err(sql_err)?;
    tx.execute(
        "INSERT INTO schema_meta (key, value) VALUES ('changes_start_after_scan', ?1)
         ON CONFLICT(key) DO NOTHING",
        params![newest.to_string()],
    )
    .map_err(sql_err)?;
    tx.commit().map_err(sql_err)
}

/// The v1.8.2 upgrade: local discovery, with nothing invented.
///
/// The two discovery tables are created for every database in [`migrate`], so
/// this migration has only one job: decide what an *existing* install starts
/// with. The answer is nothing.
///
/// # Why nothing is backfilled
///
/// Discovery evidence can only come from a multicast conversation with a
/// network. There is no historical record to derive it from — an old scan
/// recorded which ports answered, not what any device advertised — so a
/// backfill would have to invent evidence, and invented evidence is exactly
/// what this release is built to avoid. Every device simply has no discovery
/// record until the next scan of its network runs one.
///
/// Existing scans are marked `none` rather than left ambiguous, which is what
/// stops a v1.8.2 scan from being compared against a v1.8.1 one on
/// discovery-derived facts: a name that "appeared" only because the earlier
/// scan could not have seen it is not a change.
///
/// Idempotent and transactional: the update only touches rows still carrying
/// the default, so re-running it, or opening an already-current database,
/// changes nothing.
fn migrate_v5(conn: &mut Connection) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute(
        "UPDATE scans SET discovery_mode = 'none'
         WHERE discovery_mode IS NULL OR discovery_mode = ''",
        [],
    )
    .map_err(sql_err)?;
    tx.commit().map_err(sql_err)
}

/// Read the change-inbox watermark: change events exist only for scans newer
/// than this. Zero on a database that has never held a scan.
fn changes_watermark(conn: &Connection) -> Result<i64, String> {
    Ok(conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'changes_start_after_scan'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_err)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Total reported changes, which is what the UI badges count.
    fn change_count(cmp: &ScanComparison) -> usize {
        cmp.added.len() + cmp.removed.len() + cmp.changed.len()
    }

    fn host(ip: &str, mac: Option<&str>, hostname: Option<&str>, ports: &[u16]) -> HostResult {
        HostResult {
            ip: ip.into(),
            hostname: hostname.map(str::to_string),
            mac: mac.map(str::to_string),
            vendor: mac.map(|_| "Acme Networks".to_string()),
            open_ports: ports.to_vec(),
            response_ms: Some(3),
            icmp_ms: Some(2.4),
            tcp_ms: Some(3.6),
            ttl: Some(64),
            os_guess: Some("Linux/Unix/macOS".into()),
            last_seen: "2026-07-01T10:00:00+00:00".into(),
            discovery: None,
        }
    }

    fn result(target: &str, profile: Option<&str>, hosts: Vec<HostResult>) -> ScanResult {
        ScanResult {
            scan_id: 1,
            target: target.into(),
            profile: profile.map(str::to_string),
            duration_ms: 1_200,
            scanned: 254,
            probed: 254,
            hosts,
            cancelled: false,
            ports: vec![22, 80, 443],
            arp_assist: None,
            execution: None,
            scope_hint: None,
            discovery: None,
        }
    }

    // --- Discovery persistence helpers -------------------------------------

    /// A scan that ran both discovery protocols to completion.
    fn with_full_discovery(mut result: ScanResult) -> ScanResult {
        result.discovery = Some(crate::discovery::DiscoveryReport {
            mdns_attempted: true,
            ssdp_attempted: true,
            ..Default::default()
        });
        result
    }

    /// Discovery facts for one host, as the scanner would have attached them.
    fn discovery_for(
        name: &str,
        device_type: &str,
        confidence: &str,
        services: &[&str],
    ) -> crate::scanner::HostDiscovery {
        crate::scanner::HostDiscovery {
            detected_name: Some(name.into()),
            name_source: Some("mdns".into()),
            device_type: Some(device_type.into()),
            type_confidence: Some(confidence.into()),
            type_evidence: vec!["mDNS _ipp._tcp".into()],
            services: services.iter().map(|s| (*s).to_string()).collect(),
            sources: vec!["mdns".into()],
            manufacturer: Some("Acme".into()),
            model_name: Some("LaserFast 400".into()),
            ..Default::default()
        }
    }

    fn discovery_types(db: &Db) -> Vec<(String, String, String)> {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(detected_name, ''), device_type, type_confidence
                 FROM device_discovery ORDER BY device_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    fn events_of_type(db: &Db, kind: ChangeType) -> Vec<ChangeEvent> {
        db.change_events()
            .unwrap()
            .events
            .into_iter()
            .filter(|e| e.change_type == kind)
            .collect()
    }

    /// A result carrying explicit coverage, for signature-compatibility tests.
    fn result_with_ports(
        target: &str,
        profile: Option<&str>,
        ports: Vec<u16>,
        arp_assist: Option<bool>,
        hosts: Vec<HostResult>,
    ) -> ScanResult {
        let mut r = result(target, profile, hosts);
        r.ports = ports;
        r.arp_assist = arp_assist;
        r
    }

    /// A result carrying a scope hint, for network-scope tests.
    fn result_with_scope(
        target: &str,
        hosts: Vec<HostResult>,
        local_network: &str,
        gateway_mac: Option<&str>,
    ) -> ScanResult {
        let mut r = result(target, None, hosts);
        r.scope_hint = Some(crate::scanner::ScopeHint {
            local_network: Some(local_network.into()),
            gateway_ip: gateway_mac.map(|_| "192.168.1.1".into()),
            gateway_mac: gateway_mac.map(str::to_string),
            interface: Some("eth0".into()),
        });
        r
    }

    #[test]
    fn saves_and_reopens_a_scan() {
        let db = Db::open_in_memory().unwrap();
        let saved = db
            .save_scan(&result(
                "192.168.1.0/24",
                Some("quick-lan"),
                vec![
                    host(
                        "192.168.1.1",
                        Some("AA:BB:CC:00:00:01"),
                        Some("gateway"),
                        &[80, 443],
                    ),
                    host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[22]),
                ],
            ))
            .unwrap();

        let detail = db.get_scan(saved.scan_id).unwrap();
        assert_eq!(detail.summary.host_count, 2);
        assert_eq!(detail.summary.target_key, "cidr:192.168.1.0/24");
        assert_eq!(detail.summary.profile.as_deref(), Some("quick-lan"));
        assert_eq!(detail.summary.status, "completed");
        assert_eq!(detail.hosts.len(), 2);
        assert_eq!(detail.devices.len(), 2);
        assert_eq!(detail.hosts[0].open_ports, vec![80, 443]);
        // Latency is stored as two measurements plus the rounded summary.
        assert_eq!(detail.hosts[0].icmp_ms, Some(2.4));
        assert_eq!(detail.hosts[0].tcp_ms, Some(3.6));
        assert_eq!(detail.hosts[0].response_ms, Some(3));
        assert!(detail.devices[0].device_id.is_some());
    }

    #[test]
    fn first_scan_has_no_baseline_and_explains_why() {
        let db = Db::open_in_memory().unwrap();
        let saved = db
            .save_scan(&result(
                "10.0.0.0/24",
                Some("quick-lan"),
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert_eq!(change_count(&saved.comparison), 0);
        assert!(saved.comparison.baseline_scan_id.is_none());
        assert!(saved.comparison.reason.is_some());
    }

    #[test]
    fn second_scan_reports_new_missing_and_changed() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("nas"), &[445]),
                host("10.0.0.6", Some("aa:bb:cc:00:00:06"), Some("laptop"), &[]),
            ],
        ))
        .unwrap();

        let saved = db
            .save_scan(&result(
                "10.0.0.0/24",
                Some("quick-lan"),
                vec![
                    // same device, new address and an extra service
                    host(
                        "10.0.0.9",
                        Some("aa:bb:cc:00:00:05"),
                        Some("nas"),
                        &[445, 443],
                    ),
                    // brand new device
                    host("10.0.0.7", Some("aa:bb:cc:00:00:07"), Some("tablet"), &[]),
                ],
            ))
            .unwrap();

        let cmp = &saved.comparison;
        assert_eq!(cmp.added.len(), 1, "{:?}", cmp.added);
        assert_eq!(cmp.added[0].kind, ChangeKind::New);
        assert_eq!(cmp.added[0].ip, "10.0.0.7");
        assert_eq!(cmp.removed.len(), 1);
        assert_eq!(cmp.removed[0].ip, "10.0.0.6");
        assert_eq!(cmp.changed.len(), 1);
        let fields = &cmp.changed[0].fields;
        assert!(fields.iter().any(|f| f.field == "ip"));
        assert!(fields
            .iter()
            .any(|f| f.field == "ports" && f.added_ports == vec![443]));

        // The counts are stored on the scan row, so listing history is one query.
        let list = db.list_scans().unwrap();
        assert_eq!(list[0].new_count, 1);
        assert_eq!(list[0].missing_count, 1);
        assert_eq!(list[0].changed_count, 1);
        assert_eq!(list[0].baseline_scan_id, Some(list[1].id));
    }

    #[test]
    fn comparison_requires_a_compatible_target_and_coverage() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_ports(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![22, 80, 443],
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        ))
        .unwrap();

        // Different network: not a baseline.
        let other_net = db
            .save_scan(&result_with_ports(
                "192.168.5.0/24",
                Some("quick-lan"),
                vec![22, 80, 443],
                None,
                vec![host("192.168.5.5", Some("aa:bb:cc:00:00:15"), None, &[])],
            ))
            .unwrap();
        assert!(other_net.comparison.baseline_scan_id.is_none());

        // Same network, different port set: not a baseline either. Ports 80 and
        // 443 were not probed by this scan, so they must not read as closed.
        let narrower = db
            .save_scan(&result_with_ports(
                "10.0.0.0/24",
                Some("custom"),
                vec![22],
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(narrower.comparison.baseline_scan_id.is_none());

        // Same network, same ports, different discovery mode: not a baseline —
        // a routed scan cannot see ARP-only devices, so absence means nothing.
        let routed = db
            .save_scan(&result_with_ports(
                "10.0.0.0/24",
                Some("remote-subnet"),
                vec![22, 80, 443],
                Some(false),
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(routed.comparison.baseline_scan_id.is_none());

        // Same network and coverage, target written differently: matches the
        // first scan, skipping the incompatible ones in between.
        let same = db
            .save_scan(&result_with_ports(
                "10.0.0.77/24",
                Some("quick-lan"),
                vec![22, 80, 443],
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(same.comparison.baseline_scan_id.is_some());
    }

    #[test]
    fn port_order_and_duplicates_do_not_block_comparison() {
        let db = Db::open_in_memory().unwrap();
        let first = db
            .save_scan(&result_with_ports(
                "10.0.0.0/24",
                Some("custom"),
                vec![443, 22, 80],
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[443])],
            ))
            .unwrap();
        let second = db
            .save_scan(&result_with_ports(
                "10.0.0.0/24",
                Some("custom"),
                vec![22, 22, 80, 443],
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[443])],
            ))
            .unwrap();
        assert_eq!(
            second.comparison.baseline_scan_id,
            Some(first.scan_id),
            "a re-ordered, duplicated port list is the same coverage"
        );
    }

    #[test]
    fn full_tcp_scans_with_different_ranges_do_not_compare() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_ports(
            "10.0.0.0/24",
            Some("full-tcp"),
            (1..=1024).collect(),
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[80])],
        ))
        .unwrap();
        let wider = db
            .save_scan(&result_with_ports(
                "10.0.0.0/24",
                Some("full-tcp"),
                (1..=2048).collect(),
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[80])],
            ))
            .unwrap();
        assert!(wider.comparison.baseline_scan_id.is_none());
        let recheck = db.compare_scan(wider.scan_id).unwrap();
        assert_eq!(recheck.reason.as_deref(), Some(COVERAGE_MISMATCH_REASON));
    }

    #[test]
    fn compare_scan_reproduces_the_saved_comparison() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host(
                "10.0.0.5",
                Some("aa:bb:cc:00:00:05"),
                Some("nas"),
                &[445],
            )],
        ))
        .unwrap();
        let saved = db
            .save_scan(&result(
                "10.0.0.0/24",
                None,
                vec![host(
                    "10.0.0.5",
                    Some("aa:bb:cc:00:00:05"),
                    Some("nas"),
                    &[],
                )],
            ))
            .unwrap();

        let recomputed = db.compare_scan(saved.scan_id).unwrap();
        assert_eq!(recomputed.changed.len(), saved.comparison.changed.len());
        assert_eq!(
            recomputed.changed[0].fields[0].removed_ports,
            vec![445],
            "the closed port must survive a round trip through the database"
        );
    }

    #[test]
    fn device_identity_survives_a_dhcp_change() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host(
                "10.0.0.42",
                Some("aa:bb:cc:00:00:42"),
                Some("printer"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host(
                "10.0.0.57",
                Some("aa:bb:cc:00:00:42"),
                Some("printer"),
                &[80],
            )],
        ))
        .unwrap();

        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 1, "one physical device, two addresses");
        assert_eq!(devices[0].last_ip.as_deref(), Some("10.0.0.57"));
        assert_eq!(devices[0].observation_count, 2);

        let detail = db.device_detail(devices[0].id).unwrap();
        assert_eq!(detail.previous_ips, vec!["10.0.0.57", "10.0.0.42"]);
        assert!(detail.recent_changes.iter().any(|c| c.field == "ip"));
    }

    #[test]
    fn a_device_seen_without_a_mac_is_adopted_once_arp_resolves() {
        let db = Db::open_in_memory().unwrap();
        // First scan: no ARP entry yet, so the device is keyed by name + vendor.
        let mut first = host("10.0.0.30", None, Some("printer-01"), &[80]);
        first.vendor = Some("HP Inc.".into());
        db.save_scan(&result("10.0.0.0/24", None, vec![first]))
            .unwrap();
        let device_id = db.list_devices().unwrap()[0].id;
        db.set_device_name(device_id, Some("Front Office Printer".into()))
            .unwrap();

        // Second scan resolves the MAC. The existing device must be adopted, not
        // duplicated, so the operator's name and first-seen date survive.
        let mut second = host(
            "10.0.0.30",
            Some("aa:bb:cc:00:00:30"),
            Some("printer-01"),
            &[80],
        );
        second.vendor = Some("HP Inc.".into());
        db.save_scan(&result("10.0.0.0/24", None, vec![second]))
            .unwrap();

        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 1, "{devices:?}");
        assert_eq!(devices[0].id, device_id);
        assert_eq!(
            devices[0].custom_name.as_deref(),
            Some("Front Office Printer")
        );
        assert_eq!(devices[0].mac.as_deref(), Some("AA:BB:CC:00:00:30"));
        assert_eq!(devices[0].identity_source, IdentitySource::Mac);
    }

    #[test]
    fn device_name_status_and_notes_persist() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        ))
        .unwrap();
        let id = db.list_devices().unwrap()[0].id;

        db.set_device_name(id, Some("  Reception NAS  ".into()))
            .unwrap();
        db.set_device_status(id, DeviceStatus::Trusted).unwrap();
        db.set_device_notes(id, Some("Backup target".into()))
            .unwrap();

        let detail = db.device_detail(id).unwrap();
        assert_eq!(detail.device.custom_name.as_deref(), Some("Reception NAS"));
        assert_eq!(detail.device.status, DeviceStatus::Trusted);
        assert_eq!(detail.device.notes.as_deref(), Some("Backup target"));

        // Clearing a name removes it rather than storing whitespace.
        db.set_device_name(id, Some("   ".into())).unwrap();
        assert!(db.device_detail(id).unwrap().device.custom_name.is_none());

        // Editing a device that no longer exists is an error the UI can show.
        assert!(db.set_device_name(9_999, Some("x".into())).is_err());
    }

    #[test]
    fn rejects_oversized_names_and_notes() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        ))
        .unwrap();
        let id = db.list_devices().unwrap()[0].id;
        assert!(db.set_device_name(id, Some("x".repeat(121))).is_err());
        assert!(db.set_device_notes(id, Some("x".repeat(4_001))).is_err());
        assert!(db.set_device_name(id, Some("x".repeat(120))).is_ok());
    }

    #[test]
    fn deleting_a_scan_keeps_the_device_inventory() {
        let db = Db::open_in_memory().unwrap();
        let saved = db
            .save_scan(&result(
                "10.0.0.0/24",
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        let id = db.list_devices().unwrap()[0].id;
        db.set_device_name(id, Some("Keep me".into())).unwrap();

        db.delete_scan(saved.scan_id).unwrap();
        assert!(db.list_scans().unwrap().is_empty());
        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].custom_name.as_deref(), Some("Keep me"));
        assert_eq!(devices[0].observation_count, 0);
    }

    #[test]
    fn prunes_history_but_keeps_the_newest_scans() {
        let db = Db::open_in_memory().unwrap();
        for n in 1..=5u8 {
            db.save_scan(&result(
                "10.0.0.0/24",
                None,
                vec![host(
                    &format!("10.0.0.{n}"),
                    Some(&format!("aa:bb:cc:00:00:{n:02}")),
                    None,
                    &[],
                )],
            ))
            .unwrap();
        }
        assert_eq!(db.prune_history(2).unwrap(), 3);
        let remaining = db.list_scans().unwrap();
        assert_eq!(remaining.len(), 2);
        // Devices from pruned scans survive.
        assert_eq!(db.list_devices().unwrap().len(), 5);
        assert!(db.prune_history(0).is_err());
    }

    #[test]
    fn cancelled_scans_record_partial_coverage() {
        let db = Db::open_in_memory().unwrap();
        let mut partial = result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        );
        partial.cancelled = true;
        partial.probed = 40;
        let saved = db.save_scan(&partial).unwrap();
        let detail = db.get_scan(saved.scan_id).unwrap();
        assert_eq!(detail.summary.status, "cancelled");
        assert_eq!(detail.summary.probed, 40);
        assert_eq!(detail.summary.scanned, 254);
        // Partial results are still saved.
        assert_eq!(detail.hosts.len(), 1);
    }

    #[test]
    fn a_cancelled_scan_reports_no_missing_devices_or_closed_ports() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![
                host(
                    "10.0.0.5",
                    Some("aa:bb:cc:00:00:05"),
                    Some("nas"),
                    &[445, 443],
                ),
                host("10.0.0.6", Some("aa:bb:cc:00:00:06"), Some("laptop"), &[]),
            ],
        ))
        .unwrap();

        // The cancelled scan reached only the NAS, and only port 445 by the
        // time Stop landed. Nothing may be reported missing or closed.
        let mut partial = result(
            "10.0.0.0/24",
            None,
            vec![host(
                "10.0.0.5",
                Some("aa:bb:cc:00:00:05"),
                Some("nas"),
                &[445],
            )],
        );
        partial.cancelled = true;
        partial.probed = 6;
        let saved = db.save_scan(&partial).unwrap();

        let cmp = &saved.comparison;
        assert!(cmp.baseline_scan_id.is_none());
        assert!(
            cmp.removed.is_empty(),
            "no missing devices from a partial scan"
        );
        assert!(
            cmp.changed.is_empty(),
            "no closed ports from a partial scan"
        );
        assert!(cmp.added.is_empty());
        assert_eq!(cmp.reason.as_deref(), Some(PARTIAL_SCAN_REASON));

        // The stored counts agree, so history badges cannot claim changes.
        let list = db.list_scans().unwrap();
        assert_eq!(list[0].missing_count, 0);
        assert_eq!(list[0].changed_count, 0);
        assert_eq!(list[0].new_count, 0);

        // Asking again later gives the same answer.
        let recheck = db.compare_scan(saved.scan_id).unwrap();
        assert_eq!(recheck.reason.as_deref(), Some(PARTIAL_SCAN_REASON));
        assert!(recheck.baseline_scan_id.is_none());
    }

    #[test]
    fn a_cancelled_scan_never_becomes_a_baseline() {
        let db = Db::open_in_memory().unwrap();
        let full = |ip: &str| {
            result(
                "10.0.0.0/24",
                None,
                vec![host(ip, Some("aa:bb:cc:00:00:05"), Some("nas"), &[445])],
            )
        };

        let first = db.save_scan(&full("10.0.0.5")).unwrap();

        let mut partial = full("10.0.0.99");
        partial.cancelled = true;
        partial.probed = 10;
        db.save_scan(&partial).unwrap();

        // The completed scan skips the newer cancelled scan and compares with
        // the previous completed one.
        let second = db.save_scan(&full("10.0.0.5")).unwrap();
        assert_eq!(second.comparison.baseline_scan_id, Some(first.scan_id));

        // A second cancelled scan does not compare against the first one either.
        let mut partial2 = full("10.0.0.99");
        partial2.cancelled = true;
        partial2.probed = 10;
        let saved2 = db.save_scan(&partial2).unwrap();
        assert!(saved2.comparison.baseline_scan_id.is_none());
    }

    #[test]
    fn first_completed_scan_after_only_cancelled_scans_has_no_baseline() {
        let db = Db::open_in_memory().unwrap();
        for _ in 0..2 {
            let mut partial = result(
                "10.0.0.0/24",
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            );
            partial.cancelled = true;
            partial.probed = 3;
            db.save_scan(&partial).unwrap();
        }
        let completed = db
            .save_scan(&result(
                "10.0.0.0/24",
                None,
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(completed.comparison.baseline_scan_id.is_none());
        assert!(completed.comparison.reason.is_some());
    }

    #[test]
    fn same_ip_on_two_scopes_creates_two_devices() {
        let db = Db::open_in_memory().unwrap();
        // Two clients, both 192.168.1.0/24, distinguished by gateway MAC. The
        // observed host has no MAC (e.g. a routed hop), so identity falls back
        // to hostname/IP — exactly where cross-network collisions used to merge.
        let mut a_host = host("192.168.1.20", None, None, &[80]);
        a_host.vendor = None;
        let mut b_host = host("192.168.1.20", None, None, &[22]);
        b_host.vendor = None;

        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![a_host],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![b_host],
            "192.168.1.0/24",
            Some("BB:BB:BB:00:00:02"),
        ))
        .unwrap();

        let scopes = db.list_network_scopes().unwrap();
        assert_eq!(scopes.len(), 2, "two gateways mean two scopes: {scopes:?}");

        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 2, "same IP, different networks: {devices:?}");
        let scope_ids: Vec<_> = devices.iter().map(|d| d.network_scope_id).collect();
        assert_ne!(scope_ids[0], scope_ids[1]);
    }

    #[test]
    fn same_hostname_on_two_scopes_creates_two_devices() {
        let db = Db::open_in_memory().unwrap();
        let named = |ip: &str| {
            let mut h = host(ip, None, Some("office-nas"), &[445]);
            h.vendor = Some("Synology".into());
            h
        };
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![named("192.168.1.20")],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![named("192.168.1.30")],
            "192.168.1.0/24",
            Some("BB:BB:BB:00:00:02"),
        ))
        .unwrap();
        assert_eq!(db.list_devices().unwrap().len(), 2);
    }

    #[test]
    fn same_mac_on_two_scopes_does_not_mix_names_or_notes() {
        let db = Db::open_in_memory().unwrap();
        let observation = || host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[80]);

        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![observation()],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        let first_device = db.list_devices().unwrap()[0].id;
        db.set_device_name(first_device, Some("Client A printer".into()))
            .unwrap();
        db.set_device_notes(first_device, Some("Client A asset tag 42".into()))
            .unwrap();
        db.set_device_status(first_device, DeviceStatus::Trusted)
            .unwrap();

        // The same MAC appears at a different client site (cloned VM, MAC
        // randomisation, or plain coincidence): a separate device, untouched by
        // Client A's name, notes and status.
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![observation()],
            "192.168.1.0/24",
            Some("BB:BB:BB:00:00:02"),
        ))
        .unwrap();

        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 2, "{devices:?}");
        let second = devices.iter().find(|d| d.id != first_device).unwrap();
        assert!(second.custom_name.is_none());
        assert!(second.notes.is_none());
        assert_eq!(second.status, DeviceStatus::Unclassified);

        // And the first device kept everything.
        let first = db.device_detail(first_device).unwrap().device;
        assert_eq!(first.custom_name.as_deref(), Some("Client A printer"));
        assert_eq!(first.notes.as_deref(), Some("Client A asset tag 42"));
        assert_eq!(first.status, DeviceStatus::Trusted);
    }

    #[test]
    fn same_mac_within_one_scope_still_matches_across_dhcp_changes() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[80])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.57", Some("aa:bb:cc:00:00:20"), None, &[80])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 1, "one device across a DHCP change");
        assert_eq!(devices[0].last_ip.as_deref(), Some("192.168.1.57"));
    }

    #[test]
    fn a_scope_without_gateway_evidence_reuses_the_existing_scope() {
        let db = Db::open_in_memory().unwrap();
        // First scan learned the gateway; a later one could not read it (ARP
        // miss). Scope resolution must prefer continuity over a second scope.
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[])],
            "192.168.1.0/24",
            None,
        ))
        .unwrap();
        assert_eq!(db.list_network_scopes().unwrap().len(), 1);
        assert_eq!(db.list_devices().unwrap().len(), 1);
    }

    #[test]
    fn a_scope_created_without_a_gateway_adopts_one_when_learned() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[])],
            "192.168.1.0/24",
            None,
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        let scopes = db.list_network_scopes().unwrap();
        assert_eq!(scopes.len(), 1, "the naked scope adopted the gateway");
        assert_eq!(scopes[0].gateway_mac.as_deref(), Some("AA:AA:AA:00:00:01"));
    }

    #[test]
    fn a_single_host_scan_shares_the_scope_of_its_local_subnet() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[80])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        // Scanning just the printer afterwards: same physical network, so the
        // scope hint carries the same local subnet, and the device matches.
        db.save_scan(&result_with_scope(
            "192.168.1.20",
            vec![host("192.168.1.20", Some("aa:bb:cc:00:00:20"), None, &[80])],
            "192.168.1.0/24",
            Some("AA:AA:AA:00:00:01"),
        ))
        .unwrap();
        assert_eq!(db.list_network_scopes().unwrap().len(), 1);
        assert_eq!(db.list_devices().unwrap().len(), 1);
    }

    #[test]
    fn scopes_can_be_renamed_with_validation() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        ))
        .unwrap();
        let scope = &db.list_network_scopes().unwrap()[0];
        db.rename_network_scope(scope.id, "Office LAN".into())
            .unwrap();
        assert_eq!(
            db.list_network_scopes().unwrap()[0].display_name,
            "Office LAN"
        );
        assert!(db.rename_network_scope(scope.id, "   ".into()).is_err());
        assert!(db.rename_network_scope(scope.id, "x".repeat(81)).is_err());
        assert!(db.rename_network_scope(9_999, "ghost".into()).is_err());
    }

    #[test]
    fn ambiguous_generic_hostnames_do_not_merge_devices() {
        let db = Db::open_in_memory().unwrap();
        // Two MAC-less observations both calling themselves `printer`, with no
        // vendor to tell them apart, at different addresses: two devices.
        let generic = |ip: &str| {
            let mut h = host(ip, None, Some("printer"), &[9100]);
            h.vendor = None;
            h
        };
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![generic("10.0.0.40"), generic("10.0.0.41")],
        ))
        .unwrap();
        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 2, "{devices:?}");
        assert!(devices
            .iter()
            .all(|d| d.identity_source == IdentitySource::Ip));
    }

    #[test]
    fn missing_scan_and_device_lookups_report_clearly() {
        let db = Db::open_in_memory().unwrap();
        let err = db.get_scan(42).unwrap_err();
        assert!(err.contains("no longer in the history"), "{err}");
        let err = db.device_detail(42).unwrap_err();
        assert!(err.contains("no longer in the inventory"), "{err}");
        let err = db.compare_scan(42).unwrap_err();
        assert!(err.contains("no longer in the history"), "{err}");
    }

    /// Build a database with exactly the v1.6.4 schema and rows, then open it
    /// with the v1.7 migration and check nothing was lost.
    fn seed_v164(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE scans (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                target      TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                scanned     INTEGER NOT NULL
            );
            CREATE TABLE hosts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id     INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                ip          TEXT NOT NULL,
                hostname    TEXT,
                mac         TEXT,
                vendor      TEXT,
                open_ports  TEXT NOT NULL,
                response_ms INTEGER,
                ttl         INTEGER,
                os_guess    TEXT,
                last_seen   TEXT NOT NULL
            );
            CREATE INDEX idx_hosts_scan ON hosts(scan_id);

            INSERT INTO scans (id, target, created_at, duration_ms, scanned)
            VALUES (1, '192.168.1.0/24', '2026-01-05T09:00:00+00:00', 4000, 254),
                   (2, '192.168.1.0/24', '2026-02-05T09:00:00+00:00', 4200, 254);

            INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports, response_ms, ttl, os_guess, last_seen)
            VALUES (1, '192.168.1.1', 'gateway', 'A0:11:22:33:44:55', 'Acme', '80,443', 2, 64, 'Linux/Unix/macOS', '2026-01-05T09:00:00+00:00'),
                   (1, '192.168.1.40', 'printer', 'A0:11:22:33:44:66', 'HP', '80', 5, 255, 'Network device', '2026-01-05T09:00:00+00:00'),
                   (2, '192.168.1.1', 'gateway', 'A0:11:22:33:44:55', 'Acme', '80,443', 3, 64, 'Linux/Unix/macOS', '2026-02-05T09:00:00+00:00'),
                   (2, '192.168.1.55', 'printer', 'A0:11:22:33:44:66', 'HP', '80,9100', 6, 255, 'Network device', '2026-02-05T09:00:00+00:00');
            "#,
        )
        .unwrap();
    }

    #[test]
    fn upgrades_a_v164_database_without_losing_history() {
        let dir = std::env::temp_dir().join(format!("arcscan-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upgrade.db");
        let _ = std::fs::remove_file(&path);
        seed_v164(&path);

        let db = Db::open(&path).unwrap();

        // Every scan and every observation survived.
        let scans = db.list_scans().unwrap();
        assert_eq!(scans.len(), 2);
        assert_eq!(scans[0].host_count, 2);
        assert_eq!(scans[1].host_count, 2);
        // Targets were normalized so old scans are comparable.
        assert!(scans.iter().all(|s| s.target_key == "cidr:192.168.1.0/24"));
        // Old scans ran to completion, so probed was backfilled from scanned.
        assert!(scans.iter().all(|s| s.probed == 254));
        // Both scans landed in one network scope, named after their target.
        assert!(scans.iter().all(|s| s.network_scope_id.is_some()));
        assert_eq!(scans[0].network_scope_id, scans[1].network_scope_id);
        assert_eq!(scans[0].scope_name.as_deref(), Some("192.168.1.0/24"));

        // The inventory was built from the existing rows: two devices, and the
        // printer's two addresses are recognised as one device.
        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 2, "{devices:?}");
        let printer = devices
            .iter()
            .find(|d| d.mac.as_deref() == Some("A0:11:22:33:44:66"))
            .expect("printer device");
        assert_eq!(printer.observation_count, 2);
        assert_eq!(printer.first_seen, "2026-01-05T09:00:00+00:00");
        assert_eq!(printer.last_ip.as_deref(), Some("192.168.1.55"));
        assert_eq!(printer.network_scope_id, scans[0].network_scope_id);

        // v1.6 never recorded which ports a scan checked, so its scans fail
        // safely: no comparison, with the reason explained.
        let cmp = db.compare_scan(scans[0].id).unwrap();
        assert!(cmp.baseline_scan_id.is_none());
        assert_eq!(cmp.reason.as_deref(), Some(LEGACY_COVERAGE_REASON));
        assert!(cmp.added.is_empty() && cmp.removed.is_empty() && cmp.changed.is_empty());

        drop(db);

        // Re-opening runs every migration again and must change nothing.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_scans().unwrap().len(), 2);
        assert_eq!(db.list_devices().unwrap().len(), 2);
        assert_eq!(db.list_network_scopes().unwrap().len(), 1);
        // A third open, to prove idempotency is not a one-shot.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_devices().unwrap().len(), 2);
        assert_eq!(db.list_network_scopes().unwrap().len(), 1);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// Build a database with exactly the v1.7.0 (schema v2) shape and rows —
    /// global devices, no scopes, no coverage keys — then open it with the
    /// v1.7.1 migration and check nothing was lost.
    fn seed_v170(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO schema_meta (key, value) VALUES ('version', '2');

            CREATE TABLE scans (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                target      TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                scanned     INTEGER NOT NULL,
                target_key  TEXT NOT NULL DEFAULT '',
                profile     TEXT,
                probed      INTEGER NOT NULL DEFAULT 0,
                status      TEXT NOT NULL DEFAULT 'completed',
                new_count   INTEGER NOT NULL DEFAULT 0,
                missing_count INTEGER NOT NULL DEFAULT 0,
                changed_count INTEGER NOT NULL DEFAULT 0,
                baseline_scan_id INTEGER
            );
            CREATE TABLE devices (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                identity_key    TEXT NOT NULL UNIQUE,
                identity_source TEXT NOT NULL,
                mac             TEXT UNIQUE,
                custom_name     TEXT,
                hostname        TEXT,
                vendor          TEXT,
                last_ip         TEXT,
                first_seen      TEXT NOT NULL,
                last_seen       TEXT NOT NULL,
                status          TEXT NOT NULL DEFAULT 'unclassified',
                notes           TEXT
            );
            CREATE TABLE hosts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id     INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                ip          TEXT NOT NULL,
                hostname    TEXT,
                mac         TEXT,
                vendor      TEXT,
                open_ports  TEXT NOT NULL,
                response_ms INTEGER,
                ttl         INTEGER,
                os_guess    TEXT,
                last_seen   TEXT NOT NULL,
                icmp_ms     REAL,
                tcp_ms      REAL,
                device_id   INTEGER REFERENCES devices(id) ON DELETE SET NULL
            );

            INSERT INTO scans (id, target, created_at, duration_ms, scanned, target_key,
                               profile, probed, status)
            VALUES (1, '10.0.0.0/24', '2026-06-01T09:00:00+00:00', 4000, 254,
                    'cidr:10.0.0.0/24', 'quick-lan', 254, 'completed'),
                   (2, '10.0.0.0/24', '2026-06-08T09:00:00+00:00', 4100, 254,
                    'cidr:10.0.0.0/24', 'quick-lan', 254, 'completed'),
                   (3, '10.0.0.0/24', '2026-06-15T09:00:00+00:00', 3000, 254,
                    'cidr:10.0.0.0/24', 'custom', 254, 'completed');

            INSERT INTO devices (id, identity_key, identity_source, mac, custom_name,
                                 hostname, vendor, last_ip, first_seen, last_seen, status, notes)
            VALUES (1, 'mac:AA:BB:CC:00:00:01', 'mac', 'AA:BB:CC:00:00:01', 'Reception NAS',
                    'nas', 'Synology', '10.0.0.5', '2026-06-01T09:00:00+00:00',
                    '2026-06-15T09:00:00+00:00', 'trusted', 'Backup target'),
                   (2, 'mac:AA:BB:CC:00:00:02', 'mac', 'AA:BB:CC:00:00:02', NULL,
                    'laptop', 'Dell', '10.0.0.9', '2026-06-01T09:00:00+00:00',
                    '2026-06-08T09:00:00+00:00', 'known', NULL);

            INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports,
                               response_ms, last_seen, device_id)
            VALUES (1, '10.0.0.5', 'nas', 'AA:BB:CC:00:00:01', 'Synology', '445',
                    2, '2026-06-01T09:00:00+00:00', 1),
                   (1, '10.0.0.9', 'laptop', 'AA:BB:CC:00:00:02', 'Dell', '',
                    3, '2026-06-01T09:00:00+00:00', 2),
                   (2, '10.0.0.5', 'nas', 'AA:BB:CC:00:00:01', 'Synology', '445',
                    2, '2026-06-08T09:00:00+00:00', 1),
                   (2, '10.0.0.9', 'laptop', 'AA:BB:CC:00:00:02', 'Dell', '',
                    3, '2026-06-08T09:00:00+00:00', 2),
                   (3, '10.0.0.5', 'nas', 'AA:BB:CC:00:00:01', 'Synology', '445,443',
                    2, '2026-06-15T09:00:00+00:00', 1);
            "#,
        )
        .unwrap();
    }

    #[test]
    fn upgrades_a_v170_database_preserving_names_notes_and_history() {
        let dir = std::env::temp_dir().join(format!("arcscan-mig170-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upgrade170.db");
        let _ = std::fs::remove_file(&path);
        seed_v170(&path);

        let db = Db::open(&path).unwrap();

        // Every scan survived and was scoped.
        let scans = db.list_scans().unwrap();
        assert_eq!(scans.len(), 3);
        assert!(scans.iter().all(|s| s.network_scope_id.is_some()));

        // Devices kept their ids, names, notes, status and dates.
        let devices = db.list_devices().unwrap();
        assert_eq!(devices.len(), 2);
        let nas = devices.iter().find(|d| d.id == 1).expect("nas kept id 1");
        assert_eq!(nas.custom_name.as_deref(), Some("Reception NAS"));
        assert_eq!(nas.notes.as_deref(), Some("Backup target"));
        assert_eq!(nas.status, DeviceStatus::Trusted);
        assert_eq!(nas.first_seen, "2026-06-01T09:00:00+00:00");
        assert_eq!(nas.observation_count, 3);

        // Fixed-profile scans keep comparing after the upgrade: their coverage
        // was published with the profile, so it can be derived.
        let cmp = db.compare_scan(2).unwrap();
        assert_eq!(cmp.baseline_scan_id, Some(1));

        // The legacy custom scan compares with nothing, in either direction.
        let custom = db.compare_scan(3).unwrap();
        assert!(custom.baseline_scan_id.is_none());
        assert_eq!(custom.reason.as_deref(), Some(LEGACY_COVERAGE_REASON));

        drop(db);
        // Idempotent: re-opening changes nothing.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_scans().unwrap().len(), 3);
        assert_eq!(db.list_devices().unwrap().len(), 2);
        assert_eq!(db.list_network_scopes().unwrap().len(), 1);
        assert_eq!(
            db.list_devices()
                .unwrap()
                .iter()
                .find(|d| d.id == 1)
                .unwrap()
                .custom_name
                .as_deref(),
            Some("Reception NAS")
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upgrades_a_large_history_intact() {
        // A realistic long-lived inventory: 200 scans across two targets with
        // 25 devices each. Guards the v3 migration against losing rows at scale
        // and against the per-row work becoming quadratic.
        let dir = std::env::temp_dir().join(format!("arcscan-mig-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.db");
        let _ = std::fs::remove_file(&path);
        seed_v170(&path);

        {
            let conn = Connection::open(&path).unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            for scan in 4..=203i64 {
                let target = if scan % 2 == 0 {
                    ("10.0.0.0/24", "cidr:10.0.0.0/24")
                } else {
                    ("192.168.4.0/24", "cidr:192.168.4.0/24")
                };
                tx.execute(
                    "INSERT INTO scans (id, target, created_at, duration_ms, scanned, target_key,
                                        profile, probed, status)
                     VALUES (?1, ?2, ?3, 4000, 254, ?4, 'quick-lan', 254, 'completed')",
                    params![
                        scan,
                        target.0,
                        format!("2026-06-{:02}T09:00:00+00:00", scan % 28 + 1),
                        target.1
                    ],
                )
                .unwrap();
                for device in 0..25i64 {
                    tx.execute(
                        "INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports,
                                            response_ms, last_seen)
                         VALUES (?1, ?2, ?3, ?4, 'Acme', '80', 2, '2026-06-15T09:00:00+00:00')",
                        params![
                            scan,
                            format!("{}.{}", target.0.rsplit_once('.').unwrap().0, device + 10),
                            format!("host-{device}"),
                            format!(
                                "AA:BB:CC:{:02X}:{:02X}:{:02X}",
                                scan % 2,
                                device / 256,
                                device % 256
                            ),
                        ],
                    )
                    .unwrap();
                }
            }
            tx.commit().unwrap();
        }

        let began = std::time::Instant::now();
        let db = Db::open(&path).unwrap();
        let elapsed = began.elapsed();

        let scans = db.list_scans().unwrap();
        assert_eq!(scans.len(), 203);
        assert!(scans.iter().all(|s| s.network_scope_id.is_some()));
        assert!(scans.iter().all(|s| !s.coverage_key.is_empty()));
        // Two targets, so two scopes; the v1.7.0 seed shares one of them.
        assert_eq!(db.list_network_scopes().unwrap().len(), 2);
        // Every observation kept its device link.
        let orphaned: i64 = {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM hosts WHERE device_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(orphaned, 0);
        assert!(
            elapsed < std::time::Duration::from_secs(60),
            "migrating 200 scans took {elapsed:?}"
        );

        drop(db);
        // Re-opening a migrated large history must do no work at all.
        let reopened = std::time::Instant::now();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_scans().unwrap().len(), 203);
        assert!(
            reopened.elapsed() < std::time::Duration::from_secs(5),
            "re-opening re-ran the migration"
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_device_whose_scans_were_pruned_keeps_its_name_and_notes() {
        // Retention can remove every scan that observed a device while the
        // device itself survives. Migration has no scan to infer its network
        // from, so it must keep it rather than drop it or guess.
        let dir = std::env::temp_dir().join(format!("arcscan-mig-orphan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("orphan.db");
        let _ = std::fs::remove_file(&path);
        seed_v170(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "INSERT INTO devices (id, identity_key, identity_source, mac, custom_name,
                                      hostname, vendor, last_ip, first_seen, last_seen,
                                      status, notes)
                 VALUES (99, 'mac:AA:BB:CC:00:00:99', 'mac', 'AA:BB:CC:00:00:99',
                         'Retired Server', 'srv-old', 'Dell', '10.0.0.99',
                         '2025-01-01T00:00:00+00:00', '2025-06-01T00:00:00+00:00',
                         'watched', 'Decommissioned, kept for the audit trail')",
                [],
            )
            .unwrap();
        }

        let db = Db::open(&path).unwrap();
        let devices = db.list_devices().unwrap();
        let orphan = devices
            .iter()
            .find(|d| d.id == 99)
            .expect("the observation-less device survived the migration");
        assert_eq!(orphan.custom_name.as_deref(), Some("Retired Server"));
        assert_eq!(
            orphan.notes.as_deref(),
            Some("Decommissioned, kept for the audit trail")
        );
        assert_eq!(orphan.status, DeviceStatus::Watched);
        assert_eq!(orphan.first_seen, "2025-01-01T00:00:00+00:00");
        assert_eq!(orphan.observation_count, 0);
        // It is placed in a clearly-labelled scope rather than guessed into one.
        assert!(orphan.network_scope_id.is_some());
        let scope = db
            .list_network_scopes()
            .unwrap()
            .into_iter()
            .find(|s| Some(s.id) == orphan.network_scope_id)
            .unwrap();
        assert_eq!(scope.stable_key, "legacy");
        assert_eq!(scope.display_name, "Earlier inventory");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn adopts_v16_device_labels_from_local_storage() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![
                host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[]),
                host("10.0.0.6", Some("aa:bb:cc:00:00:06"), None, &[]),
            ],
        ))
        .unwrap();

        let mut labels = HashMap::new();
        // v1.6 stored uppercase colon-separated keys; accept any spelling.
        labels.insert("AA:BB:CC:00:00:05".to_string(), "Reception NAS".to_string());
        // An empty label meant "starred, unnamed".
        labels.insert("aa-bb-cc-00-00-06".to_string(), String::new());
        labels.insert("not-a-mac".to_string(), "ignored".to_string());

        assert_eq!(db.import_device_labels(labels.clone()).unwrap(), 2);

        let devices = db.list_devices().unwrap();
        let nas = devices
            .iter()
            .find(|d| d.mac.as_deref() == Some("AA:BB:CC:00:00:05"))
            .unwrap();
        assert_eq!(nas.custom_name.as_deref(), Some("Reception NAS"));
        assert_eq!(nas.status, DeviceStatus::Known);
        let starred = devices
            .iter()
            .find(|d| d.mac.as_deref() == Some("AA:BB:CC:00:00:06"))
            .unwrap();
        assert!(starred.custom_name.is_none());
        assert_eq!(starred.status, DeviceStatus::Known);

        // Re-importing must not clobber a name edited since.
        db.set_device_name(nas.id, Some("Reception NAS 2".into()))
            .unwrap();
        db.import_device_labels(labels).unwrap();
        let devices = db.list_devices().unwrap();
        let nas = devices.iter().find(|d| d.id == nas.id).unwrap();
        assert_eq!(nas.custom_name.as_deref(), Some("Reception NAS 2"));
    }

    #[test]
    fn last_scan_ips_returns_the_newest_scan_only() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.1", Some("AA:BB:CC:00:00:01"), None, &[])],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            None,
            vec![
                host("10.0.0.2", Some("aa:bb:cc:00:00:02"), None, &[]),
                host("10.0.0.3", Some("aa:bb:cc:00:00:03"), None, &[]),
            ],
        ))
        .unwrap();
        let mut ips = db.last_scan_ips().unwrap();
        ips.sort();
        assert_eq!(ips, vec!["10.0.0.2", "10.0.0.3"]);
    }

    // -----------------------------------------------------------------------
    // v1.8: persistent inventory
    // -----------------------------------------------------------------------

    /// Find one inventory row by its current or most recent address.
    fn row_at<'a>(summary: &'a InventorySummary, ip: &str) -> &'a InventoryRow {
        summary
            .rows
            .iter()
            .find(|r| r.current_ip.as_deref() == Some(ip))
            .unwrap_or_else(|| panic!("no inventory row for {ip}"))
    }

    #[test]
    fn inventory_summarises_every_device_across_scans() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host(
                    "10.0.0.1",
                    Some("AA:BB:CC:00:00:01"),
                    Some("gateway"),
                    &[80],
                ),
                host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("nas"), &[445]),
            ],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host(
                    "10.0.0.1",
                    Some("AA:BB:CC:00:00:01"),
                    Some("gateway"),
                    &[80],
                ),
                // The NAS moved and opened HTTPS.
                host(
                    "10.0.0.9",
                    Some("aa:bb:cc:00:00:05"),
                    Some("nas"),
                    &[445, 443],
                ),
            ],
        ))
        .unwrap();

        let summary = db.inventory().unwrap();
        assert_eq!(
            summary.rows.len(),
            2,
            "one row per device, not per sighting"
        );

        let nas = row_at(&summary, "10.0.0.9");
        assert_eq!(nas.display_name, "nas");
        assert_eq!(nas.mac.as_deref(), Some("AA:BB:CC:00:00:05"));
        assert_eq!(nas.observation_count, 2);
        assert_eq!(nas.open_ports, vec![445, 443]);
        // The address it used to hold is carried without the current one.
        assert_eq!(nas.previous_ips, vec!["10.0.0.5"]);
        assert!(!nas.notes_present);
        assert_eq!(nas.presence, PresenceState::Present);
        assert_eq!(nas.latest_icmp_ms, Some(2.4));

        assert_eq!(summary.present, 2);
        assert_eq!(summary.missing, 0);
        assert_eq!(summary.unknown, 0);
        assert!(!summary.needs_completed_scan);
    }

    #[test]
    fn inventory_marks_a_device_missing_only_from_a_completed_compatible_scan() {
        let db = Db::open_in_memory().unwrap();
        let both = vec![
            host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gateway"),
                &[80],
            ),
            host(
                "10.0.0.7",
                Some("aa:bb:cc:00:00:07"),
                Some("printer"),
                &[631],
            ),
        ];
        db.save_scan(&result("10.0.0.0/24", Some("quick-lan"), both))
            .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gateway"),
                &[80],
            )],
        ))
        .unwrap();

        let summary = db.inventory().unwrap();
        assert_eq!(
            row_at(&summary, "10.0.0.1").presence,
            PresenceState::Present
        );
        assert_eq!(
            row_at(&summary, "10.0.0.7").presence,
            PresenceState::Missing
        );
        assert_eq!(summary.present, 1);
        assert_eq!(summary.missing, 1);
        // A missing device keeps every fact the inventory holds about it.
        let printer = row_at(&summary, "10.0.0.7");
        assert_eq!(printer.observation_count, 1);
        assert_eq!(printer.open_ports, vec![631]);
    }

    #[test]
    fn a_partial_scan_never_marks_a_device_missing() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host(
                    "10.0.0.1",
                    Some("AA:BB:CC:00:00:01"),
                    Some("gateway"),
                    &[80],
                ),
                host(
                    "10.0.0.7",
                    Some("aa:bb:cc:00:00:07"),
                    Some("printer"),
                    &[631],
                ),
            ],
        ))
        .unwrap();

        // A scan stopped part-way that happened to miss the printer.
        let mut partial = result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gateway"),
                &[80],
            )],
        );
        partial.cancelled = true;
        partial.probed = 90;
        db.save_scan(&partial).unwrap();

        let summary = db.inventory().unwrap();
        // Presence still comes from the last *completed* scan, which saw both.
        assert_eq!(
            row_at(&summary, "10.0.0.7").presence,
            PresenceState::Present
        );
        assert_eq!(summary.missing, 0);
        assert_eq!(summary.present, 2);

        // And the partial scan created no change events at all.
        let feed = db.change_events().unwrap();
        assert_eq!(feed.total, 0, "{:?}", feed.events);
    }

    #[test]
    fn presence_is_unknown_without_a_completed_scan() {
        let db = Db::open_in_memory().unwrap();
        let mut partial = result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host("10.0.0.1", Some("AA:BB:CC:00:00:01"), None, &[80])],
        );
        partial.cancelled = true;
        db.save_scan(&partial).unwrap();

        let summary = db.inventory().unwrap();
        assert_eq!(summary.rows.len(), 1);
        assert_eq!(summary.rows[0].presence, PresenceState::Unknown);
        assert_eq!(summary.unknown, 1);
        assert!(summary.rows[0].last_completed_scan_id.is_none());
        assert!(
            summary.needs_completed_scan,
            "the UI must be able to say a completed scan is required"
        );
    }

    #[test]
    fn presence_is_unknown_when_only_a_different_coverage_ever_saw_the_device() {
        let db = Db::open_in_memory().unwrap();
        // A wide sweep sees the printer.
        db.save_scan(&result_with_ports(
            "10.0.0.0/24",
            Some("full-tcp"),
            vec![22, 80, 443, 631],
            Some(true),
            vec![
                host(
                    "10.0.0.1",
                    Some("AA:BB:CC:00:00:01"),
                    Some("gateway"),
                    &[80],
                ),
                host(
                    "10.0.0.7",
                    Some("aa:bb:cc:00:00:07"),
                    Some("printer"),
                    &[631],
                ),
            ],
        ))
        .unwrap();
        // A later, narrower scan does not. Its coverage differs, so the printer's
        // absence proves nothing and must not read as Missing.
        db.save_scan(&result_with_ports(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![80],
            Some(true),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gateway"),
                &[80],
            )],
        ))
        .unwrap();

        let summary = db.inventory().unwrap();
        assert_eq!(
            row_at(&summary, "10.0.0.1").presence,
            PresenceState::Present
        );
        assert_eq!(
            row_at(&summary, "10.0.0.7").presence,
            PresenceState::Unknown
        );
        assert_eq!(summary.missing, 0);
    }

    #[test]
    fn inventory_keeps_networks_apart_and_offers_them_as_filters() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host(
                "192.168.1.10",
                Some("aa:bb:cc:00:00:10"),
                Some("laptop"),
                &[],
            )],
            "192.168.1.0/24",
            Some("11:11:11:11:11:11"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host(
                "192.168.1.10",
                Some("aa:bb:cc:00:00:10"),
                Some("laptop"),
                &[],
            )],
            "192.168.1.0/24",
            Some("22:22:22:22:22:22"),
        ))
        .unwrap();

        let scopes = db.list_network_scopes().unwrap();
        assert_eq!(scopes.len(), 2, "duplicate private ranges must stay apart");
        db.rename_network_scope(scopes[0].id, "Office".into())
            .unwrap();

        let summary = db.inventory().unwrap();
        assert_eq!(
            summary.rows.len(),
            2,
            "the same MAC on two networks is two devices"
        );
        assert_eq!(summary.networks.len(), 2);
        assert_eq!(
            summary.networks.iter().map(|n| n.device_count).sum::<i64>(),
            2
        );
        assert!(
            summary.networks.iter().any(|n| n.name == "Office"),
            "a renamed network reaches the inventory filter: {:?}",
            summary.networks
        );
        assert!(summary
            .rows
            .iter()
            .any(|r| r.network_name.as_deref() == Some("Office")));
    }

    #[test]
    fn inventory_reports_the_operator_label_notes_flag_and_status() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.5",
                Some("aa:bb:cc:00:00:05"),
                Some("nas"),
                &[445],
            )],
        ))
        .unwrap();
        let id = db.list_devices().unwrap()[0].id;
        db.set_device_name(id, Some("Backup NAS".into())).unwrap();
        db.set_device_notes(id, Some("Nightly backups".into()))
            .unwrap();
        db.set_device_status(id, DeviceStatus::Trusted).unwrap();

        let summary = db.inventory().unwrap();
        let row = &summary.rows[0];
        assert_eq!(row.display_name, "Backup NAS");
        assert_eq!(row.custom_name.as_deref(), Some("Backup NAS"));
        assert_eq!(row.status, DeviceStatus::Trusted);
        assert!(row.notes_present, "the indicator must be set");
        // The row carries no note body: the drawer loads that for one device.
        assert_eq!(row.hostname.as_deref(), Some("nas"));
    }

    #[test]
    fn an_inventory_of_five_thousand_devices_is_two_queries_and_stays_quick() {
        let db = Db::open_in_memory().unwrap();
        // 5,000 devices over 20 scans is 100,000 observations, which is the
        // shape the release is meant to hold up under.
        let mut hosts = Vec::with_capacity(5_000);
        for i in 0..5_000u32 {
            let octets = [10, 1 + (i / 65_536) as u8, (i / 256) as u8, (i % 256) as u8];
            hosts.push(host(
                &format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3]),
                Some(&format!(
                    "aa:bb:{:02x}:{:02x}:{:02x}:{:02x}",
                    octets[0], octets[1], octets[2], octets[3]
                )),
                Some(&format!("host-{i}")),
                &[80, 443],
            ));
        }
        for _ in 0..20 {
            db.save_scan(&result("10.0.0.0/8", Some("quick-lan"), hosts.clone()))
                .unwrap();
        }

        let started = std::time::Instant::now();
        let summary = db.inventory().unwrap();
        let elapsed = started.elapsed();

        assert_eq!(summary.rows.len(), 5_000);
        assert_eq!(summary.present, 5_000);
        assert_eq!(summary.rows[0].observation_count, 20);
        // Generous, because CI machines vary; the point is that it is bounded
        // work rather than 5,000 round trips, which would take far longer.
        assert!(
            elapsed.as_secs() < 20,
            "inventory of 100,000 observations took {elapsed:?}"
        );
    }

    // -----------------------------------------------------------------------
    // v1.8: change events
    // -----------------------------------------------------------------------

    /// Change types present in a feed, for readable assertions.
    fn kinds(feed: &ChangeFeed) -> Vec<ChangeType> {
        feed.events.iter().map(|e| e.change_type).collect()
    }

    #[test]
    fn a_completed_scan_records_one_event_per_change() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("nas"), &[445]),
                host(
                    "10.0.0.7",
                    Some("aa:bb:cc:00:00:07"),
                    Some("printer"),
                    &[631],
                ),
            ],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                // Moved, renamed and opened HTTPS while closing SMB.
                host(
                    "10.0.0.6",
                    Some("aa:bb:cc:00:00:05"),
                    Some("nas-01"),
                    &[443],
                ),
                // Brand new.
                host("10.0.0.9", Some("aa:bb:cc:00:00:09"), Some("laptop"), &[]),
            ],
        ))
        .unwrap();

        let feed = db.change_events().unwrap();
        let found = kinds(&feed);
        for expected in [
            ChangeType::DeviceAdded,
            ChangeType::DeviceMissing,
            ChangeType::IpChanged,
            ChangeType::HostnameChanged,
            ChangeType::PortsChanged,
        ] {
            assert!(
                found.contains(&expected),
                "{expected:?} missing from {found:?}"
            );
        }
        assert_eq!(feed.unreviewed, feed.total);

        // Port changes keep the structured lists, not just display text.
        let ports = feed
            .events
            .iter()
            .find(|e| e.change_type == ChangeType::PortsChanged)
            .unwrap();
        assert_eq!(ports.opened_ports, vec![443]);
        assert_eq!(ports.closed_ports, vec![445]);
        assert!(ports.new_value.as_deref().unwrap().contains("443"));

        // Every event names the scan and baseline it came from, so the inbox can
        // link to both comparisons.
        for event in &feed.events {
            assert!(event.scan_id.is_some());
            assert!(event.baseline_scan_id.is_some());
            assert!(event.scan_at.is_some());
            assert!(event.baseline_at.is_some());
        }
    }

    #[test]
    fn a_returning_device_is_recorded_as_returned_not_added() {
        let db = Db::open_in_memory().unwrap();
        let phone = host("10.0.0.20", Some("aa:bb:cc:00:00:20"), Some("phone"), &[]);
        let gateway = host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]);
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![gateway.clone(), phone.clone()],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![gateway.clone()],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![gateway, phone],
        ))
        .unwrap();

        let feed = db.change_events().unwrap();
        assert_eq!(feed.events[0].change_type, ChangeType::DeviceReturned);
        assert_eq!(feed.events[1].change_type, ChangeType::DeviceMissing);
        assert_eq!(feed.total, 2);
    }

    #[test]
    fn saving_the_same_comparison_twice_creates_no_duplicate_events() {
        let db = Db::open_in_memory().unwrap();
        let first = result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        );
        db.save_scan(&first).unwrap();
        let second = result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        );
        let saved = db.save_scan(&second).unwrap();
        let before = db.change_events().unwrap().total;
        assert_eq!(before, 1);

        // Re-record the identical comparison, the way a retried save would.
        {
            let mut conn = db.lock().unwrap();
            let tx = conn.transaction().unwrap();
            let baseline = BaselineScan {
                id: saved.comparison.baseline_scan_id.unwrap(),
                target: "10.0.0.0/24".into(),
                created_at: saved.comparison.baseline_created_at.clone().unwrap(),
                discovery_mode: "none".into(),
            };
            let written = record_change_events(
                &tx,
                saved.scan_id,
                1,
                &baseline,
                &saved.comparison,
                "2026-08-03T10:00:00+00:00",
            )
            .unwrap();
            tx.commit().unwrap();
            assert_eq!(written, 0, "a repeated save must write nothing");
        }
        assert_eq!(db.change_events().unwrap().total, before);
    }

    #[test]
    fn acknowledging_and_reopening_an_event_keeps_it() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        ))
        .unwrap();

        let feed = db.change_events().unwrap();
        let id = feed.events[0].id;
        let outcome = db
            .set_change_state(&[id], ChangeState::Acknowledged)
            .unwrap();
        assert_eq!(outcome.updated, 1);
        assert!(outcome.missing.is_empty());

        let feed = db.change_events().unwrap();
        assert_eq!(feed.events[0].state, ChangeState::Acknowledged);
        assert!(feed.events[0].acknowledged_at.is_some());
        assert_eq!(feed.unreviewed, 0);
        assert_eq!(feed.total, 1, "acknowledging must never delete the record");

        // Undo puts it back and clears the stamp rather than leaving a stale one.
        db.set_change_state(&[id], ChangeState::Unreviewed).unwrap();
        let feed = db.change_events().unwrap();
        assert_eq!(feed.events[0].state, ChangeState::Unreviewed);
        assert!(feed.events[0].acknowledged_at.is_none());
        assert_eq!(feed.unreviewed, 1);
    }

    #[test]
    fn a_bulk_action_reports_ids_that_no_longer_exist() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        ))
        .unwrap();
        let id = db.change_events().unwrap().events[0].id;

        let outcome = db
            .set_change_state(&[id, 9_999], ChangeState::Acknowledged)
            .unwrap();
        assert_eq!(outcome.updated, 1);
        assert_eq!(outcome.missing, vec![9_999]);
    }

    #[test]
    fn ignoring_a_device_hides_its_changes_without_losing_them() {
        let db = Db::open_in_memory().unwrap();
        let gateway = host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]);
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![gateway.clone()],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                gateway.clone(),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        ))
        .unwrap();

        let tv = db
            .inventory()
            .unwrap()
            .rows
            .iter()
            .find(|r| r.hostname.as_deref() == Some("tv"))
            .unwrap()
            .device_id;

        let outcome = db
            .set_device_statuses(&[tv], DeviceStatus::Ignored)
            .unwrap();
        assert_eq!(outcome.updated, 1);

        // The existing event left the default inbox but is still recorded.
        let feed = db.change_events().unwrap();
        assert_eq!(feed.total, 1);
        assert_eq!(feed.unreviewed, 0);
        assert_eq!(feed.events[0].state, ChangeState::Ignored);

        // A later change to the same device is recorded already ignored.
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                gateway,
                host(
                    "10.0.0.4",
                    Some("aa:bb:cc:00:00:04"),
                    Some("tv"),
                    &[8009, 8443],
                ),
            ],
        ))
        .unwrap();
        let feed = db.change_events().unwrap();
        assert_eq!(feed.total, 2);
        assert_eq!(feed.unreviewed, 0, "an ignored device must not reappear");
        assert!(feed.events.iter().all(|e| e.state == ChangeState::Ignored));

        // Ignoring never removes the device or its history.
        let summary = db.inventory().unwrap();
        assert_eq!(summary.rows.len(), 2);
        let row = summary.rows.iter().find(|r| r.device_id == tv).unwrap();
        assert_eq!(row.status, DeviceStatus::Ignored);
        assert_eq!(row.observation_count, 2);
    }

    #[test]
    fn renaming_a_device_updates_its_change_events() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        ))
        .unwrap();

        let feed = db.change_events().unwrap();
        assert_eq!(feed.events[0].device_label, "tv");
        let device_id = feed.events[0].device_id.unwrap();
        db.set_device_name(device_id, Some("Lounge TV".into()))
            .unwrap();

        let feed = db.change_events().unwrap();
        assert_eq!(feed.events[0].device_label, "Lounge TV");
        assert_eq!(
            feed.events[0].device_status,
            Some(DeviceStatus::Unclassified)
        );
    }

    #[test]
    fn change_events_survive_the_scan_that_produced_them_being_deleted() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        let second = db
            .save_scan(&result(
                "10.0.0.0/24",
                Some("quick-lan"),
                vec![
                    host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                    host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
                ],
            ))
            .unwrap();

        db.delete_scan(second.scan_id).unwrap();
        let feed = db.change_events().unwrap();
        assert_eq!(feed.total, 1, "pruning history must not erase the record");
        // The date the change was found is kept with the event itself.
        assert!(feed.events[0].scan_at.is_some());
        assert_eq!(feed.events[0].device_label, "tv");
        // The link is cleared rather than left pointing at a scan that is gone,
        // so "Open the scan" says so instead of failing.
        assert!(feed.events[0].scan_id.is_none());

        // The inventory is intact too.
        assert_eq!(db.inventory().unwrap().rows.len(), 2);
    }

    #[test]
    fn retention_keeps_change_records_and_clears_their_dead_links() {
        let db = Db::open_in_memory().unwrap();
        let gateway = host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]);
        for extra in [None, Some(4u8), Some(5), Some(6)] {
            let mut hosts = vec![gateway.clone()];
            if let Some(n) = extra {
                hosts.push(host(
                    &format!("10.0.0.{n}"),
                    Some(&format!("aa:bb:cc:00:00:0{n}")),
                    Some("device"),
                    &[8009],
                ));
            }
            db.save_scan(&result("10.0.0.0/24", Some("quick-lan"), hosts))
                .unwrap();
        }
        let before = db.change_events().unwrap();
        assert!(before.total > 0);

        // Keep only the newest scan; every earlier one goes.
        let removed = db.prune_history(1).unwrap();
        assert_eq!(removed, 3);

        let after = db.change_events().unwrap();
        assert_eq!(
            after.total, before.total,
            "retention must not erase records"
        );
        let survivor = db.list_scans().unwrap()[0].id;
        for event in &after.events {
            assert!(
                event.scan_id.is_none_or(|id| id == survivor),
                "event {} still points at a deleted scan",
                event.id
            );
            assert!(event.baseline_scan_id.is_none_or(|id| id == survivor));
            assert!(event.scan_at.is_some(), "the date is kept on the record");
        }
        // Devices, names and dates are untouched by retention.
        assert_eq!(db.inventory().unwrap().rows.len(), 4);
    }

    #[test]
    fn device_detail_carries_presence_network_and_change_events() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host(
                "192.168.1.4",
                Some("aa:bb:cc:00:00:04"),
                Some("tv"),
                &[8009],
            )],
            "192.168.1.0/24",
            Some("11:11:11:11:11:11"),
        ))
        .unwrap();
        db.save_scan(&result_with_scope(
            "192.168.1.0/24",
            vec![host(
                "192.168.1.4",
                Some("aa:bb:cc:00:00:04"),
                Some("tv"),
                &[8009, 8443],
            )],
            "192.168.1.0/24",
            Some("11:11:11:11:11:11"),
        ))
        .unwrap();

        let scope = db.list_network_scopes().unwrap()[0].id;
        db.rename_network_scope(scope, "Home Wi-Fi".into()).unwrap();

        let device_id = db.list_devices().unwrap()[0].id;
        let detail = db.device_detail(device_id).unwrap();
        assert_eq!(detail.presence, PresenceState::Present);
        assert_eq!(detail.network_name.as_deref(), Some("Home Wi-Fi"));
        assert_eq!(detail.events.len(), 1);
        assert_eq!(detail.events[0].change_type, ChangeType::PortsChanged);
        assert_eq!(detail.events[0].opened_ports, vec![8443]);
    }

    #[test]
    fn drawer_presence_agrees_with_the_inventory_query() {
        // Two implementations of one rule only stay honest if a test compares
        // them; this covers present, missing and unknown in one database.
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("AA:BB:CC:00:00:01"), Some("gw"), &[80]),
                host(
                    "10.0.0.7",
                    Some("aa:bb:cc:00:00:07"),
                    Some("printer"),
                    &[631],
                ),
            ],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host(
                "10.0.0.1",
                Some("AA:BB:CC:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        // A second network with nothing but a stopped scan.
        let mut partial = result_with_scope(
            "172.16.0.0/24",
            vec![host("172.16.0.5", Some("aa:bb:cc:00:00:55"), None, &[])],
            "172.16.0.0/24",
            Some("33:33:33:33:33:33"),
        );
        partial.cancelled = true;
        db.save_scan(&partial).unwrap();

        let summary = db.inventory().unwrap();
        assert!(summary.rows.len() >= 3);
        for row in &summary.rows {
            let detail = db.device_detail(row.device_id).unwrap();
            assert_eq!(
                detail.presence, row.presence,
                "drawer and inventory disagree for device {}",
                row.device_id
            );
        }
        assert_eq!(summary.present, 1);
        assert_eq!(summary.missing, 1);
        assert_eq!(summary.unknown, 1);
    }

    // -----------------------------------------------------------------------
    // v1.8: migration from v1.7.1
    // -----------------------------------------------------------------------

    /// A v1.7.1 database (schema v3): scoped devices, coverage keys, existing
    /// comparisons, names, notes, a partial scan and a legacy-coverage scan.
    fn seed_v171(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO schema_meta (key, value) VALUES ('version', '3');

            CREATE TABLE network_scopes (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                stable_key       TEXT NOT NULL UNIQUE,
                display_name     TEXT NOT NULL,
                canonical_target TEXT,
                gateway_mac      TEXT,
                interface_hint   TEXT,
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL
            );
            CREATE TABLE scans (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                target      TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                scanned     INTEGER NOT NULL,
                target_key  TEXT NOT NULL DEFAULT '',
                profile     TEXT,
                probed      INTEGER NOT NULL DEFAULT 0,
                status      TEXT NOT NULL DEFAULT 'completed',
                new_count   INTEGER NOT NULL DEFAULT 0,
                missing_count INTEGER NOT NULL DEFAULT 0,
                changed_count INTEGER NOT NULL DEFAULT 0,
                baseline_scan_id INTEGER,
                network_scope_id INTEGER REFERENCES network_scopes(id),
                coverage_key TEXT NOT NULL DEFAULT '',
                execution_settings TEXT
            );
            CREATE TABLE devices (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                network_scope_id INTEGER NOT NULL REFERENCES network_scopes(id),
                identity_key     TEXT NOT NULL,
                identity_source  TEXT NOT NULL,
                mac              TEXT,
                custom_name      TEXT,
                hostname         TEXT,
                vendor           TEXT,
                last_ip          TEXT,
                first_seen       TEXT NOT NULL,
                last_seen        TEXT NOT NULL,
                status           TEXT NOT NULL DEFAULT 'unclassified',
                notes            TEXT,
                UNIQUE(network_scope_id, identity_key),
                UNIQUE(network_scope_id, mac)
            );
            CREATE TABLE hosts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id     INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
                ip          TEXT NOT NULL,
                hostname    TEXT,
                mac         TEXT,
                vendor      TEXT,
                open_ports  TEXT NOT NULL,
                response_ms INTEGER,
                ttl         INTEGER,
                os_guess    TEXT,
                last_seen   TEXT NOT NULL,
                icmp_ms     REAL,
                tcp_ms      REAL,
                device_id   INTEGER REFERENCES devices(id) ON DELETE SET NULL
            );

            -- Two networks that reuse the same private range, told apart by their
            -- gateways, plus one named and one left automatic.
            INSERT INTO network_scopes (id, stable_key, display_name, canonical_target,
                                        gateway_mac, created_at, updated_at)
            VALUES (1, 'target:cidr:192.168.1.0/24|gw:11:11:11:11:11:11', 'Home Wi-Fi',
                    'cidr:192.168.1.0/24', '11:11:11:11:11:11',
                    '2026-05-01T09:00:00+00:00', '2026-07-01T09:00:00+00:00'),
                   (2, 'target:cidr:192.168.1.0/24|gw:22:22:22:22:22:22', '192.168.1.0/24',
                    'cidr:192.168.1.0/24', '22:22:22:22:22:22',
                    '2026-05-02T09:00:00+00:00', '2026-07-02T09:00:00+00:00');

            INSERT INTO scans (id, target, created_at, duration_ms, scanned, target_key, profile,
                               probed, status, new_count, missing_count, changed_count,
                               baseline_scan_id, network_scope_id, coverage_key)
            VALUES (1, '192.168.1.0/24', '2026-06-01T09:00:00+00:00', 4000, 254,
                    'cidr:192.168.1.0/24', 'quick-lan', 254, 'completed', 0, 0, 0, NULL, 1,
                    'v1|arp:auto|ports:22,80,443'),
                   (2, '192.168.1.0/24', '2026-06-08T09:00:00+00:00', 4100, 254,
                    'cidr:192.168.1.0/24', 'quick-lan', 254, 'completed', 1, 1, 1, 1, 1,
                    'v1|arp:auto|ports:22,80,443'),
                   -- A scan stopped early: never a baseline, never a reference.
                   (3, '192.168.1.0/24', '2026-06-09T09:00:00+00:00', 900, 254,
                    'cidr:192.168.1.0/24', 'quick-lan', 60, 'cancelled', 0, 0, 0, NULL, 1,
                    'v1|arp:auto|ports:22,80,443'),
                   -- The other network, recorded before coverage keys existed.
                   (4, '192.168.1.0/24', '2026-06-10T09:00:00+00:00', 4000, 254,
                    'cidr:192.168.1.0/24', 'custom', 254, 'completed', 0, 0, 0, NULL, 2,
                    'legacy:custom:4');

            INSERT INTO devices (id, network_scope_id, identity_key, identity_source, mac,
                                 custom_name, hostname, vendor, last_ip, first_seen, last_seen,
                                 status, notes)
            VALUES (1, 1, 'mac:AA:BB:CC:00:00:01', 'mac', 'AA:BB:CC:00:00:01', 'Office Printer',
                    'printer', 'HP Inc.', '192.168.1.7', '2026-06-01T09:00:00+00:00',
                    '2026-06-01T09:00:00+00:00', 'known', 'Toner reordered automatically'),
                   (2, 1, 'mac:AA:BB:CC:00:00:02', 'mac', 'AA:BB:CC:00:00:02', NULL,
                    'laptop', 'Dell Inc.', '192.168.1.20', '2026-06-01T09:00:00+00:00',
                    '2026-06-08T09:00:00+00:00', 'trusted', NULL),
                   -- No MAC at all: identified by hostname and vendor.
                   (3, 1, 'hv:camera-01|axis', 'hostname-vendor', NULL, NULL,
                    'camera-01', 'Axis', '192.168.1.30', '2026-06-08T09:00:00+00:00',
                    '2026-06-08T09:00:00+00:00', 'unclassified', NULL),
                   -- Same MAC as device 1, different network. Must stay separate.
                   (4, 2, 'mac:AA:BB:CC:00:00:01', 'mac', 'AA:BB:CC:00:00:01', NULL,
                    'printer', 'HP Inc.', '192.168.1.7', '2026-06-10T09:00:00+00:00',
                    '2026-06-10T09:00:00+00:00', 'unclassified', NULL);

            INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports, response_ms,
                               last_seen, device_id)
            VALUES (1, '192.168.1.7', 'printer', 'AA:BB:CC:00:00:01', 'HP Inc.', '80,631',
                    4, '2026-06-01T09:00:00+00:00', 1),
                   (1, '192.168.1.11', 'laptop', 'AA:BB:CC:00:00:02', 'Dell Inc.', '',
                    3, '2026-06-01T09:00:00+00:00', 2),
                   -- Second scan: the printer is gone, the laptop moved, a camera arrived.
                   (2, '192.168.1.20', 'laptop', 'AA:BB:CC:00:00:02', 'Dell Inc.', '445',
                    3, '2026-06-08T09:00:00+00:00', 2),
                   (2, '192.168.1.30', 'camera-01', NULL, 'Axis', '80,554',
                    9, '2026-06-08T09:00:00+00:00', 3),
                   (3, '192.168.1.20', 'laptop', 'AA:BB:CC:00:00:02', 'Dell Inc.', '445',
                    3, '2026-06-09T09:00:00+00:00', 2),
                   (4, '192.168.1.7', 'printer', 'AA:BB:CC:00:00:01', 'HP Inc.', '80',
                    4, '2026-06-10T09:00:00+00:00', 4);
            "#,
        )
        .unwrap();
    }

    #[test]
    fn upgrades_a_v171_database_without_creating_a_review_backlog() {
        let dir = std::env::temp_dir().join(format!("arcscan-mig171-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upgrade171.db");
        let _ = std::fs::remove_file(&path);
        seed_v171(&path);

        let db = Db::open(&path).unwrap();

        // The point of the migration decision: nothing to review on first launch.
        let feed = db.change_events().unwrap();
        assert_eq!(feed.total, 0, "an upgrade must not invent a backlog");
        assert_eq!(feed.unreviewed, 0);
        assert_eq!(
            feed.starts_after_scan_id, 4,
            "the watermark records where the inbox begins"
        );

        // Everything v1.7.1 held is still here.
        assert_eq!(db.list_scans().unwrap().len(), 4);
        assert_eq!(db.list_network_scopes().unwrap().len(), 2);

        let summary = db.inventory().unwrap();
        assert_eq!(summary.rows.len(), 4);
        assert_eq!(summary.networks.len(), 2);

        let printer = summary
            .rows
            .iter()
            .find(|r| r.custom_name.as_deref() == Some("Office Printer"))
            .expect("the named device survived");
        assert_eq!(printer.status, DeviceStatus::Known);
        assert!(printer.notes_present);
        assert_eq!(printer.first_seen, "2026-06-01T09:00:00+00:00");
        // Scan 2 is the reference for Home Wi-Fi and did not see the printer,
        // and scan 1 shares its coverage, so this is a real Missing.
        assert_eq!(printer.presence, PresenceState::Missing);
        assert_eq!(printer.last_completed_scan_id, Some(2));

        let laptop = summary
            .rows
            .iter()
            .find(|r| r.hostname.as_deref() == Some("laptop"))
            .unwrap();
        assert_eq!(laptop.presence, PresenceState::Present);
        assert_eq!(laptop.status, DeviceStatus::Trusted);
        assert_eq!(laptop.current_ip.as_deref(), Some("192.168.1.20"));
        assert_eq!(laptop.previous_ips, vec!["192.168.1.11"]);
        assert_eq!(laptop.observation_count, 3);

        // A device with no MAC keeps its hostname-and-vendor identity.
        let camera = summary
            .rows
            .iter()
            .find(|r| r.hostname.as_deref() == Some("camera-01"))
            .unwrap();
        assert!(camera.mac.is_none());
        assert_eq!(camera.identity_source, IdentitySource::HostnameVendor);
        assert_eq!(camera.presence, PresenceState::Present);

        // The second network only ever had a legacy-coverage scan, so nothing
        // there can be called present or missing.
        let other = summary
            .rows
            .iter()
            .find(|r| r.network_scope_id == Some(2))
            .unwrap();
        assert_eq!(other.presence, PresenceState::Unknown);
        assert_eq!(
            other.network_name.as_deref(),
            Some("192.168.1.0/24"),
            "an unnamed network keeps its automatic label"
        );

        // Reopening the database changes nothing: the migration is idempotent.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.change_events().unwrap().starts_after_scan_id, 4);
        assert_eq!(db.inventory().unwrap().rows.len(), 4);
    }

    #[test]
    fn the_first_scan_after_an_upgrade_records_its_changes() {
        let dir = std::env::temp_dir().join(format!("arcscan-mig171b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upgrade171b.db");
        let _ = std::fs::remove_file(&path);
        seed_v171(&path);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.change_events().unwrap().total, 0);

        // A new scan of Home Wi-Fi with exactly the coverage the history used.
        let mut next = result_with_ports(
            "192.168.1.0/24",
            Some("quick-lan"),
            vec![22, 80, 443],
            None,
            vec![host(
                "192.168.1.20",
                Some("aa:bb:cc:00:00:02"),
                Some("laptop"),
                &[445, 3389],
            )],
        );
        next.scope_hint = Some(crate::scanner::ScopeHint {
            local_network: Some("192.168.1.0/24".into()),
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: Some("11:11:11:11:11:11".into()),
            interface: Some("eth0".into()),
        });
        db.save_scan(&next).unwrap();

        let feed = db.change_events().unwrap();
        assert!(feed.total > 0, "post-upgrade scans do record changes");
        assert!(kinds(&feed).contains(&ChangeType::PortsChanged));
        assert!(kinds(&feed).contains(&ChangeType::DeviceMissing));
        assert_eq!(feed.unreviewed, feed.total);
    }

    #[test]
    fn a_fresh_database_starts_its_inbox_at_zero() {
        let db = Db::open_in_memory().unwrap();
        let feed = db.change_events().unwrap();
        assert_eq!(feed.starts_after_scan_id, 0);
        assert_eq!(feed.total, 0);
        let summary = db.inventory().unwrap();
        assert!(summary.rows.is_empty());
        assert!(
            !summary.needs_completed_scan,
            "an empty inventory asks for a first scan, not for a completed one"
        );
    }

    // --- Discovery persistence (v1.8.2) ------------------------------------

    #[test]
    fn discovery_evidence_is_stored_and_reaches_the_inventory_and_the_drawer() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host(
            "10.0.0.5",
            Some("aa:bb:cc:00:00:05"),
            Some("printer-01"),
            &[631],
        );
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();

        let row = &db.inventory().unwrap().rows[0];
        let discovery = row.discovery.as_ref().expect("the row carries discovery");
        assert_eq!(discovery.detected_name.as_deref(), Some("Studio Printer"));
        assert_eq!(discovery.device_type, "printer");
        assert_eq!(discovery.type_confidence, "high");
        assert_eq!(discovery.services, vec!["_ipp._tcp"]);
        // The detected name outranks the reverse-DNS hostname for display.
        assert_eq!(row.display_name, "Studio Printer");

        let detail = db.device_detail(row.device_id).unwrap();
        let record = detail.discovery.expect("the drawer gets the full record");
        assert_eq!(record.manufacturer.as_deref(), Some("Acme"));
        assert_eq!(record.model_name.as_deref(), Some("LaserFast 400"));
        assert!(record.evidence.iter().any(|e| e.kind == "service"));
        assert!(record.evidence.iter().all(|e| !e.first_seen.is_empty()));
    }

    // --- v1.8.3: the shape of the work at scale ----------------------------

    /// Build a database at the scale the release notes claim to have measured:
    /// 5,000 devices, 100,000 observations, 50,000 evidence rows, 1,000 type
    /// corrections and 1,000 devices whose evidence has gone stale.
    ///
    /// Written with raw inserts rather than through `save_scan`, because the
    /// point is to measure the *read* paths against a large database, not to
    /// measure how long it takes to create one.
    fn seed_at_scale(path: &std::path::Path) {
        const DEVICES: i64 = 5_000;
        const SCANS: i64 = 20;
        const EVIDENCE_PER_DEVICE: i64 = 10;

        {
            // Create the current schema through the real migration, so the
            // fixture cannot drift from the shape the app actually uses.
            let _ = Db::open(path).unwrap();
        }
        let mut conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        let tx = conn.transaction().unwrap();

        tx.execute(
            "INSERT INTO network_scopes (id, stable_key, display_name, canonical_target,
                                         created_at, updated_at)
             VALUES (1, 'scale', 'Scale', '10.0.0.0/16',
                     '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();

        for scan in 1..=SCANS {
            tx.execute(
                "INSERT INTO scans (id, target, created_at, duration_ms, scanned, target_key,
                                    profile, probed, status, network_scope_id, coverage_key,
                                    discovery_mode)
                 VALUES (?1, '10.0.0.0/16', ?2, 4000, 65536, 'cidr:10.0.0.0/16',
                         'quick-lan', 65536, 'completed', 1, 'v1|ports:22,80,443', 'full')",
                params![scan, format!("2026-02-{:02}T09:00:00+00:00", scan)],
            )
            .unwrap();
        }

        {
            let mut device = tx
                .prepare(
                    "INSERT INTO devices (id, network_scope_id, identity_key, identity_source, mac,
                                          custom_name, hostname, vendor, last_ip, first_seen,
                                          last_seen, status, user_device_type)
                     VALUES (?1, 1, ?2, 'mac', ?3, NULL, ?4, 'Example Corp', ?5,
                             '2026-02-01T09:00:00+00:00', '2026-02-20T09:00:00+00:00',
                             'unclassified', ?6)",
                )
                .unwrap();
            let mut discovery = tx
                .prepare(
                    "INSERT INTO device_discovery (device_id, network_scope_id, detected_name,
                                                   name_source, device_type, type_confidence,
                                                   services, sources, first_discovered_at,
                                                   last_discovered_at, naming_rules_version)
                     VALUES (?1, 1, ?2, 'mdns', 'printer', 'high', '[\"_ipp._tcp\"]',
                             '[\"mdns\"]', '2026-02-01T09:00:00+00:00',
                             '2026-02-20T09:00:00+00:00', ?3)",
                )
                .unwrap();
            let mut evidence = tx
                .prepare(
                    "INSERT INTO discovery_evidence (device_id, network_scope_id, source, kind, key,
                                                     value, normalized_value, confidence,
                                                     first_seen, last_seen, last_scan_id, misses)
                     VALUES (?1, 1, 'mdns', ?2, ?3, ?4, ?4, 'high',
                             '2026-02-01T09:00:00+00:00', '2026-02-20T09:00:00+00:00', 20, ?5)",
                )
                .unwrap();
            let mut host = tx
                .prepare(
                    "INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports,
                                        response_ms, last_seen, device_id, ttl, os_guess)
                     VALUES (?1, ?2, ?3, ?4, 'Example Corp', '22,80,443', 3, ?5, ?6, 64, 'Linux')",
                )
                .unwrap();

            for id in 1..=DEVICES {
                let mac = format!(
                    "AA:BB:{:02X}:{:02X}:{:02X}:{:02X}",
                    id >> 24,
                    (id >> 16) & 0xFF,
                    (id >> 8) & 0xFF,
                    id & 0xFF
                );
                let ip = format!("10.0.{}.{}", id / 254, id % 254 + 1);
                let hostname = format!("device-{id}");
                // 1,000 devices carry an operator correction, and a different
                // 1,000 have evidence that has gone stale.
                let override_type = (id % 5 == 0).then_some("television");
                let misses = if id % 5 == 1 { 4 } else { 0 };

                device
                    .execute(params![
                        id,
                        format!("mac:{mac}"),
                        mac,
                        hostname,
                        ip,
                        override_type
                    ])
                    .unwrap();
                discovery
                    .execute(params![id, format!("Device {id}"), NAMING_RULES_VERSION])
                    .unwrap();
                for n in 0..EVIDENCE_PER_DEVICE {
                    evidence
                        .execute(params![
                            id,
                            "service",
                            format!("_svc{n}._tcp"),
                            format!("_svc{n}._tcp"),
                            misses
                        ])
                        .unwrap();
                }
                for scan in 1..=SCANS {
                    host.execute(params![
                        scan,
                        ip,
                        hostname,
                        mac,
                        format!("2026-02-{:02}T09:00:00+00:00", scan),
                        id
                    ])
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }

    #[test]
    fn the_inventory_and_the_drawer_stay_fast_on_a_large_database() {
        // 5,000 devices, 100,000 observations and 50,000 evidence rows, with
        // 1,000 type corrections and 1,000 devices whose evidence is stale.
        //
        // The bounds below are deliberately loose: this runs on shared CI
        // hardware and the point is to catch a *change of shape* — a query per
        // device, or a scan of all evidence per row — not to police tens of
        // milliseconds.
        //
        // The inventory bound matches the one the v1.8.2 test above uses for
        // the same 5,000 devices and 100,000 observations, and for the same
        // reason: most of that time is the window function picking each
        // device's latest observation, which is seconds on any machine and
        // several seconds on a slow one. A tighter bound does not measure
        // shape, it measures the runner — an earlier 6,000 ms limit here read
        // 4.4 s locally and 6.6 s on CI, and failed for no defect. What it is
        // built to catch is far bigger than the spread between runners: 5,000
        // round trips against this database cost tens of seconds.
        let dir = std::env::temp_dir().join(format!("arcscan-scale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scale.db");
        let _ = std::fs::remove_file(&path);
        seed_at_scale(&path);

        let db = Db::open(&path).unwrap();

        let started = std::time::Instant::now();
        let inventory = db.inventory().unwrap();
        let inventory_ms = started.elapsed().as_millis();
        assert_eq!(inventory.rows.len(), 5_000);
        assert!(
            inventory_ms < 20_000,
            "the inventory took {inventory_ms} ms, which suggests per-device work"
        );

        // Every state is actually present, or the measurement is of the wrong
        // thing.
        let corrected = inventory
            .rows
            .iter()
            .filter(|r| r.user_device_type.is_some())
            .count();
        assert_eq!(corrected, 1_000);
        let stale = inventory
            .rows
            .iter()
            .filter(|r| {
                r.discovery
                    .as_ref()
                    .is_some_and(|d| d.evidence_freshness == "stale")
            })
            .count();
        assert_eq!(stale, 1_000);
        // And the freshness reduction reached them: a high-confidence type on
        // wholly stale evidence reads as medium.
        assert!(inventory.rows.iter().any(|r| {
            r.discovery
                .as_ref()
                .is_some_and(|d| d.evidence_freshness == "stale" && d.type_confidence == "medium")
        }));

        // Opening a device panel: the full record, its evidence and its history.
        let device_id = inventory.rows[0].device_id;
        let started = std::time::Instant::now();
        let detail = db.device_detail(device_id).unwrap();
        let drawer_ms = started.elapsed().as_millis();
        assert!(detail.discovery.is_some());
        assert!(
            drawer_ms < 1_500,
            "opening a device took {drawer_ms} ms on a 50,000-row evidence table"
        );

        // Building a diagnostic report reads one device, not the whole table.
        let started = std::time::Instant::now();
        let report = db.device_discovery_report(device_id, "1.8.3").unwrap();
        let report_ms = started.elapsed().as_millis();
        assert!(report.contains("ArcScan discovery report"));
        assert!(
            report_ms < 1_500,
            "building a diagnostic report took {report_ms} ms"
        );

        // Setting a correction is one row, whatever the size of the database.
        let started = std::time::Instant::now();
        db.set_device_type_override(device_id, Some("printer".into()))
            .unwrap();
        let write_ms = started.elapsed().as_millis();
        assert!(
            write_ms < 1_000,
            "one correction took {write_ms} ms, which suggests it is not one row"
        );

        // And it recorded nothing in the inbox, at any size.
        assert!(db.change_events().unwrap().events.is_empty());

        println!(
            "scale: inventory {inventory_ms} ms, drawer {drawer_ms} ms, \
             report {report_ms} ms, correction {write_ms} ms"
        );
        let _ = std::fs::remove_file(&path);
    }

    // --- v1.8.3: user type overrides ---------------------------------------

    /// One saved device with a detected type, ready to be corrected.
    fn device_with_detected_type(db: &Db, detected: &str, confidence: &str) -> i64 {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[80]);
        host.discovery = Some(discovery_for(
            "Living Room TV",
            detected,
            confidence,
            &["_airplay._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();
        db.inventory().unwrap().rows[0].device_id
    }

    #[test]
    fn a_device_with_no_override_reads_as_auto() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "media_device", "medium");
        let row = &db.inventory().unwrap().rows[0];
        assert_eq!(row.user_device_type, None);
        assert_eq!(row.discovery.as_ref().unwrap().device_type, "media_device");
        assert_eq!(db.device_detail(id).unwrap().device.user_device_type, None);
    }

    #[test]
    fn every_shipped_type_can_be_chosen_as_an_override() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "media_device", "medium");
        for kind in crate::discovery::DeviceType::ALL {
            db.set_device_type_override(id, Some(kind.as_str().into()))
                .unwrap();
            let row = &db.inventory().unwrap().rows[0];
            assert_eq!(row.user_device_type.as_deref(), Some(kind.as_str()));
            // The detected answer is kept underneath, every time.
            assert_eq!(row.discovery.as_ref().unwrap().device_type, "media_device");
        }
    }

    #[test]
    fn an_explicit_unknown_override_is_stored_and_is_not_the_same_as_clearing() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "camera", "medium");

        db.set_device_type_override(id, Some("unknown".into()))
            .unwrap();
        assert_eq!(
            db.inventory().unwrap().rows[0].user_device_type.as_deref(),
            Some("unknown")
        );

        db.set_device_type_override(id, None).unwrap();
        assert_eq!(db.inventory().unwrap().rows[0].user_device_type, None);
        // And clearing revealed the automatic answer rather than erasing it.
        assert_eq!(
            db.inventory().unwrap().rows[0]
                .discovery
                .as_ref()
                .unwrap()
                .device_type,
            "camera"
        );
    }

    #[test]
    fn an_override_that_is_not_a_device_type_is_refused_rather_than_stored() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "printer", "high");
        for bogus in [
            "toaster",
            "",
            "Printer",
            "media device",
            "'; DROP TABLE devices; --",
        ] {
            let error = db
                .set_device_type_override(id, Some(bogus.into()))
                .expect_err("a value that is not a type must be refused");
            assert!(error.contains("not a device type"), "{error}");
        }
        // Nothing was stored, and in particular nothing became an explicit
        // Unknown, which is a real answer nobody gave.
        assert_eq!(db.inventory().unwrap().rows[0].user_device_type, None);
    }

    #[test]
    fn an_override_changes_nothing_about_the_device_but_its_type() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "media_device", "medium");
        db.set_device_name(id, Some("The Big TV".into())).unwrap();
        db.set_device_notes(id, Some("Behind the sofa".into()))
            .unwrap();
        db.set_device_status(id, DeviceStatus::Trusted).unwrap();

        let before = db.device_detail(id).unwrap();
        let before_evidence = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        let before_events = db.change_events().unwrap().events.len();

        db.set_device_type_override(id, Some("television".into()))
            .unwrap();

        let after = db.device_detail(id).unwrap();
        // Identity, scope, presence, trust, name, notes and dates: untouched.
        assert_eq!(after.device.id, before.device.id);
        assert_eq!(after.device.identity_key, before.device.identity_key);
        assert_eq!(after.device.identity_source, before.device.identity_source);
        assert_eq!(after.device.mac, before.device.mac);
        assert_eq!(
            after.device.network_scope_id,
            before.device.network_scope_id
        );
        assert_eq!(after.device.custom_name, before.device.custom_name);
        assert_eq!(after.device.notes, before.device.notes);
        assert_eq!(after.device.status, before.device.status);
        assert_eq!(after.device.first_seen, before.device.first_seen);
        assert_eq!(after.device.last_seen, before.device.last_seen);
        assert_eq!(after.presence, before.presence);
        assert_eq!(
            after.device.observation_count,
            before.device.observation_count
        );
        // Exactly one device: an override never creates a second one.
        assert_eq!(db.inventory().unwrap().rows.len(), 1);
        // Discovery evidence and the detected answer are untouched.
        let after_evidence = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        assert_eq!(after_evidence, before_evidence);
        assert_eq!(
            after.discovery.as_ref().unwrap().device_type,
            before.discovery.as_ref().unwrap().device_type
        );
        // And no change event was recorded: an operator edit is not a network
        // event, and putting one in the inbox would be noise.
        assert_eq!(db.change_events().unwrap().events.len(), before_events);
    }

    #[test]
    fn an_override_survives_a_later_scan_that_detects_something_else() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "media_device", "medium");
        db.set_device_type_override(id, Some("television".into()))
            .unwrap();

        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[80]);
        host.discovery = Some(discovery_for(
            "Living Room TV",
            "speaker",
            "high",
            &["_raop._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();

        let row = &db.inventory().unwrap().rows[0];
        assert_eq!(row.user_device_type.as_deref(), Some("television"));
        // The automatic answer moved on underneath, which is what clearing the
        // override has to be able to reveal.
        assert_eq!(row.discovery.as_ref().unwrap().device_type, "speaker");
    }

    // --- v1.8.3: evidence aging -------------------------------------------

    /// The miss count on one device's service claims, lowest first.
    fn evidence_misses(db: &Db, device_id: i64) -> Vec<i64> {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT misses FROM discovery_evidence
                 WHERE device_id = ?1 AND kind = 'service' ORDER BY value",
            )
            .unwrap();
        let rows = stmt.query_map([device_id], |r| r.get::<_, i64>(0)).unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    /// A host advertising `services`, with the same identity every time.
    fn advertising(services: &[&str]) -> HostResult {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[80]);
        host.discovery = Some(discovery_for(
            "Living Room TV",
            "media_device",
            "medium",
            services,
        ));
        host
    }

    #[test]
    fn three_qualifying_misses_make_a_claim_stale_and_fresh_evidence_resets_it() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
        )))
        .unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;

        let freshness_of = |db: &Db, value: &str| -> String {
            db.device_detail(id)
                .unwrap()
                .discovery
                .unwrap()
                .evidence
                .into_iter()
                .find(|e| e.value == value)
                .map(|e| e.freshness)
                .unwrap_or_default()
        };

        assert_eq!(freshness_of(&db, "_raop._tcp"), "current");

        // Three completed, both-protocol scans in which the device answered but
        // stopped advertising `_raop._tcp`.
        for expected in ["aging", "aging", "stale"] {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![advertising(&["_airplay._tcp"])],
            )))
            .unwrap();
            assert_eq!(freshness_of(&db, "_raop._tcp"), expected);
            // The claim it *did* re-hear stayed current throughout.
            assert_eq!(freshness_of(&db, "_airplay._tcp"), "current");
        }

        // Stale evidence is kept, not deleted, and keeps its dates.
        let stale = db
            .device_detail(id)
            .unwrap()
            .discovery
            .unwrap()
            .evidence
            .into_iter()
            .find(|e| e.value == "_raop._tcp")
            .expect("stale evidence is still on file");
        assert!(!stale.first_seen.is_empty());
        assert!(!stale.last_seen.is_empty());
        assert_eq!(stale.misses, 3);

        // One sighting puts it straight back to current.
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
        )))
        .unwrap();
        assert_eq!(freshness_of(&db, "_raop._tcp"), "current");
    }

    #[test]
    fn nothing_ages_on_a_scan_that_did_not_qualify() {
        // Each case is one reason a scan must not count as a miss, applied to a
        // device that answered it and stopped advertising one of its services.
        /// One reason a scan must not count as a qualifying miss, as a name
        /// and the change that makes an otherwise-qualifying scan into it.
        type NonQualifying = (&'static str, Box<dyn Fn(ScanResult) -> ScanResult>);

        let cases: Vec<NonQualifying> = vec![
            (
                "a stopped scan",
                Box::new(|mut r: ScanResult| {
                    r.cancelled = true;
                    r
                }),
            ),
            (
                "a scan interrupted during discovery",
                Box::new(|mut r: ScanResult| {
                    r.discovery = Some(crate::discovery::DiscoveryReport {
                        mdns_attempted: true,
                        ssdp_attempted: true,
                        interrupted: true,
                        ..Default::default()
                    });
                    r
                }),
            ),
            (
                "a scan with only one protocol",
                Box::new(|mut r: ScanResult| {
                    r.discovery = Some(crate::discovery::DiscoveryReport {
                        mdns_attempted: true,
                        ssdp_attempted: false,
                        ..Default::default()
                    });
                    r
                }),
            ),
            (
                "a scan with discovery switched off or skipped",
                Box::new(|mut r: ScanResult| {
                    r.discovery = Some(crate::discovery::DiscoveryReport::skipped(
                        "Local discovery is switched off",
                    ));
                    r
                }),
            ),
            (
                "a scan that recorded no discovery at all",
                Box::new(|mut r: ScanResult| {
                    r.discovery = None;
                    r
                }),
            ),
        ];

        for (what, degrade) in cases {
            let db = Db::open_in_memory().unwrap();
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
            )))
            .unwrap();
            let id = db.inventory().unwrap().rows[0].device_id;
            assert_eq!(evidence_misses(&db, id), vec![0, 0], "{what}: setup");

            for _ in 0..4 {
                db.save_scan(&degrade(result(
                    "10.0.0.0/24",
                    None,
                    vec![advertising(&["_airplay._tcp"])],
                )))
                .unwrap();
            }
            assert_eq!(
                evidence_misses(&db, id),
                vec![0, 0],
                "{what} aged evidence and must not have"
            );
        }
    }

    #[test]
    fn a_device_the_scan_never_found_does_not_age() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
        )))
        .unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;

        // Four completed, both-protocol scans of the same network in which the
        // device was simply not there. A device that was switched off has not
        // stopped advertising.
        for _ in 0..4 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![host("10.0.0.9", Some("aa:bb:cc:00:00:09"), None, &[80])],
            )))
            .unwrap();
        }
        assert_eq!(evidence_misses(&db, id), vec![0, 0]);
        assert_eq!(
            db.inventory()
                .unwrap()
                .rows
                .iter()
                .find(|r| r.device_id == id)
                .unwrap()
                .discovery
                .as_ref()
                .unwrap()
                .evidence_freshness,
            "current"
        );
    }

    #[test]
    fn description_fields_do_not_age_when_no_description_was_read() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[80]);
        host.discovery = Some(discovery_for(
            "Living Room TV",
            "media_device",
            "medium",
            &["_airplay._tcp"],
        ));
        // The first scan read a description, so the manufacturer and model are
        // on file as SSDP claims.
        let mut first = with_full_discovery(result("10.0.0.0/24", None, vec![host.clone()]));
        first.discovery.as_mut().unwrap().descriptions_fetched = 1;
        db.save_scan(&first).unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;

        // Four later scans ran both protocols but read no description — the
        // setting is off, or no device offered one. That is not evidence the
        // manufacturer changed.
        let mut quiet = host.clone();
        quiet.discovery.as_mut().unwrap().manufacturer = None;
        quiet.discovery.as_mut().unwrap().model_name = None;
        for _ in 0..4 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![quiet.clone()],
            )))
            .unwrap();
        }

        let misses: Vec<i64> = {
            let conn = db.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT misses FROM discovery_evidence
                     WHERE device_id = ?1 AND kind IN ('manufacturer', 'model')",
                )
                .unwrap();
            let rows = stmt.query_map([id], |r| r.get::<_, i64>(0)).unwrap();
            rows.collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert!(!misses.is_empty(), "the description claims are on file");
        assert!(
            misses.iter().all(|m| *m == 0),
            "description fields aged with no description read: {misses:?}"
        );
    }

    #[test]
    fn stale_evidence_can_no_longer_carry_high_confidence_on_its_own() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;
        assert_eq!(
            db.inventory().unwrap().rows[0]
                .discovery
                .as_ref()
                .unwrap()
                .type_confidence,
            "high"
        );

        // Three qualifying scans in which the device answered and advertised
        // nothing at all.
        let mut silent =
            super::tests::host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[631]);
        silent.discovery = Some(crate::scanner::HostDiscovery::default());
        for _ in 0..3 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![silent.clone()],
            )))
            .unwrap();
        }

        let row = &db.inventory().unwrap().rows[0];
        let discovery = row.discovery.as_ref().unwrap();
        assert_eq!(discovery.evidence_freshness, "stale");
        assert_eq!(discovery.type_confidence, "medium");
        // Still a printer: it stopped answering, it did not become something
        // else, and there is no decay past Medium.
        assert_eq!(discovery.device_type, "printer");

        let detail = db.device_detail(id).unwrap().discovery.unwrap();
        assert_eq!(detail.type_confidence, "medium");
        // The classifier's own answer is kept so the drawer can explain the
        // reduction rather than only show it.
        assert_eq!(detail.raw_type_confidence, "high");
        assert_eq!(detail.evidence_freshness, "stale");
    }

    #[test]
    fn evidence_aging_records_no_change_event() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
        )))
        .unwrap();
        // Two scans that still hear everything, to establish a baseline and let
        // any first-scan suppression fall away.
        for _ in 0..2 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![advertising(&["_airplay._tcp", "_raop._tcp"])],
            )))
            .unwrap();
        }
        let baseline: Vec<String> = db
            .change_events()
            .unwrap()
            .events
            .iter()
            .map(|e| e.change_type.as_str().to_string())
            .collect();

        // Now let one claim age all the way to stale. The *service* going away
        // is a real v1.8.2 event and is expected once; nothing further may be
        // recorded as the counter keeps climbing.
        for _ in 0..4 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![advertising(&["_airplay._tcp"])],
            )))
            .unwrap();
        }

        let after: Vec<String> = db
            .change_events()
            .unwrap()
            .events
            .iter()
            .map(|e| e.change_type.as_str().to_string())
            .collect();
        let added: Vec<&str> = after
            .iter()
            .skip(baseline.len().min(after.len()))
            .map(String::as_str)
            .collect();
        assert!(
            added.iter().all(|kind| *kind == "service_disappeared"),
            "aging produced events other than the one service going away: {added:?}"
        );
        // And in particular no type change: the stored classification never
        // moved, only the miss counter did.
        assert!(
            !after.iter().any(|k| k == "device_type_changed"),
            "aging changed the device type: {after:?}"
        );
    }

    // --- v1.8.3: the upgrade is quiet --------------------------------------

    #[test]
    fn the_first_scan_after_the_naming_rules_changed_records_no_rename() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("thing"), &[80]);
        host.discovery = Some(discovery_for(
            "Old Name",
            "media_device",
            "high",
            &["_airplay._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host.clone()],
        )))
        .unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;

        // Stand the stored record back down to the generation v1.8.2 wrote,
        // which is exactly what an upgraded database looks like.
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "UPDATE device_discovery SET naming_rules_version = 0 WHERE device_id = ?1",
                [id],
            )
            .unwrap();
        }
        let before = db.change_events().unwrap().events.len();

        // The first scan under the new rules reads the name differently.
        let mut renamed = host.clone();
        renamed.discovery.as_mut().unwrap().detected_name = Some("Tidied Name".into());
        renamed.discovery.as_mut().unwrap().model_name = Some("Acme LaserFast 400".into());
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![renamed.clone()],
        )))
        .unwrap();

        let events = db.change_events().unwrap().events;
        assert_eq!(
            events.len(),
            before,
            "the upgrade produced a rename backlog: {:?}",
            events
                .iter()
                .map(|e| (e.change_type.as_str(), e.new_value.clone()))
                .collect::<Vec<_>>()
        );
        // The tidied name was still adopted; it was only the *event* that was
        // suppressed.
        assert_eq!(
            db.inventory().unwrap().rows[0]
                .discovery
                .as_ref()
                .unwrap()
                .detected_name
                .as_deref(),
            Some("Tidied Name")
        );

        // And suppression is once only: a genuine rename on the next scan is
        // reported normally.
        let mut again = renamed;
        again.discovery.as_mut().unwrap().detected_name = Some("Genuinely Renamed".into());
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![again],
        )))
        .unwrap();
        assert!(
            db.change_events()
                .unwrap()
                .events
                .iter()
                .any(|e| e.change_type.as_str() == "detected_name_changed"),
            "the second scan should report a real rename"
        );
    }

    #[test]
    fn a_v182_database_upgrades_without_losing_anything_and_reopens_clean() {
        let dir = std::env::temp_dir().join(format!("arcscan-mig183-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upgrade-183.db");
        let _ = std::fs::remove_file(&path);

        // Build a v1.8.2-shaped database: current tables, then the two v1.8.3
        // columns removed and the version stood back down.
        {
            let db = Db::open(&path).unwrap();
            let mut host = host(
                "10.0.0.5",
                Some("aa:bb:cc:00:00:05"),
                Some("printer-01"),
                &[631],
            );
            host.discovery = Some(discovery_for(
                "Studio Printer",
                "printer",
                "high",
                &["_ipp._tcp"],
            ));
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![host],
            )))
            .unwrap();
            let id = db.inventory().unwrap().rows[0].device_id;
            db.set_device_name(id, Some("Front Office Printer".into()))
                .unwrap();
            db.set_device_notes(id, Some("Third floor".into())).unwrap();
            db.set_device_status(id, DeviceStatus::Trusted).unwrap();
        }
        let (before_rows, before_evidence, before_events, before_identity) = {
            let db = Db::open(&path).unwrap();
            let rows = db.inventory().unwrap().rows;
            let conn = db.lock().unwrap();
            let evidence: i64 = conn
                .query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| r.get(0))
                .unwrap();
            drop(conn);
            let events = db.change_events().unwrap().events.len();
            let identity: Vec<(i64, String)> = rows
                .iter()
                .map(|r| {
                    (
                        r.device_id,
                        db.device_detail(r.device_id).unwrap().device.identity_key,
                    )
                })
                .collect();
            (rows.len(), evidence, events, identity)
        };

        // Stand the schema back down to 5, the way a real v1.8.2 file is.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "ALTER TABLE devices DROP COLUMN user_device_type;
                 ALTER TABLE device_discovery DROP COLUMN naming_rules_version;
                 UPDATE schema_meta SET value = '5' WHERE key = 'version';",
            )
            .unwrap();
        }

        // Upgrade.
        let db = Db::open(&path).unwrap();
        let rows = db.inventory().unwrap().rows;
        assert_eq!(rows.len(), before_rows);
        assert_eq!(rows[0].custom_name.as_deref(), Some("Front Office Printer"));
        assert_eq!(rows[0].status, DeviceStatus::Trusted);
        assert!(rows[0].notes_present);
        // Nothing was reclassified, nothing was re-keyed, and no upgrade
        // backlog appeared in the inbox.
        assert_eq!(rows[0].discovery.as_ref().unwrap().device_type, "printer");
        assert_eq!(db.change_events().unwrap().events.len(), before_events);
        for (id, key) in &before_identity {
            assert_eq!(&db.device_detail(*id).unwrap().device.identity_key, key);
        }
        {
            let conn = db.lock().unwrap();
            let evidence: i64 = conn
                .query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| r.get(0))
                .unwrap();
            assert_eq!(evidence, before_evidence);
        }
        // Existing evidence starts safely: nothing arrives already stale.
        assert_eq!(
            rows[0].discovery.as_ref().unwrap().evidence_freshness,
            "current"
        );
        // Auto, not an explicit Unknown: an upgrade makes no choices for the
        // operator.
        assert_eq!(rows[0].user_device_type, None);

        // Reopening changes nothing further.
        drop(db);
        let reopened = Db::open(&path).unwrap();
        let again = reopened.inventory().unwrap().rows;
        assert_eq!(again.len(), before_rows);
        assert_eq!(
            again[0].custom_name.as_deref(),
            Some("Front Office Printer")
        );
        assert_eq!(
            reopened.change_events().unwrap().events.len(),
            before_events
        );
        let version: String = {
            let conn = reopened.lock().unwrap();
            conn.query_row(
                "SELECT value FROM schema_meta WHERE key = 'version'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn an_override_survives_the_upgrade_path_and_a_reopen() {
        let dir = std::env::temp_dir().join(format!("arcscan-ovr183-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("override-183.db");
        let _ = std::fs::remove_file(&path);
        let id = {
            let db = Db::open(&path).unwrap();
            let id = device_with_detected_type(&db, "media_device", "medium");
            db.set_device_type_override(id, Some("unknown".into()))
                .unwrap();
            id
        };
        let db = Db::open(&path).unwrap();
        assert_eq!(
            db.device_detail(id)
                .unwrap()
                .device
                .user_device_type
                .as_deref(),
            Some("unknown")
        );
        assert_eq!(
            db.inventory().unwrap().rows[0].user_device_type.as_deref(),
            Some("unknown")
        );
    }

    // --- v1.8.3: the diagnostic report -------------------------------------

    #[test]
    fn the_discovery_report_is_useful_and_carries_nothing_identifying() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host(
            "192.168.1.42",
            Some("aa:bb:cc:00:00:05"),
            Some("printer-01"),
            &[631],
        );
        let mut discovery = discovery_for("Studio Printer", "printer", "high", &["_ipp._tcp"]);
        discovery.serial_number = Some("SN-DEADBEEF".into());
        discovery.presentation_url = Some("http://192.168.1.42:8080/index.html".into());
        host.discovery = Some(discovery);
        db.save_scan(&with_full_discovery(result(
            "192.168.1.0/24",
            None,
            vec![host],
        )))
        .unwrap();
        let id = db.inventory().unwrap().rows[0].device_id;
        db.set_device_notes(id, Some("The finance printer, do not move".into()))
            .unwrap();
        db.set_device_name(id, Some("Finance Printer".into()))
            .unwrap();

        let report = db.device_discovery_report(id, "1.8.3").unwrap();

        // Useful.
        for expected in [
            "ArcScan discovery report",
            "Version: 1.8.3",
            "Device type: Printer",
            "Type source: Automatic",
            "Studio Printer",
            "_ipp._tcp",
        ] {
            assert!(report.contains(expected), "missing {expected:?}:\n{report}");
        }

        // And carrying nothing that identifies the unit or its owner.
        for forbidden in [
            "SN-DEADBEEF",
            "aa:bb:cc:00:00:05",
            "AA:BB:CC:00:00:05",
            "192.168.1.42",
            "index.html",
            "do not move",
            "Finance Printer",
        ] {
            assert!(
                !report.contains(forbidden),
                "the report leaked {forbidden:?}:\n{report}"
            );
        }
        assert!(report.contains("192.168.x.x"));

        // Deterministic and bounded.
        assert_eq!(report, db.device_discovery_report(id, "1.8.3").unwrap());
        assert!(report.chars().count() <= crate::discovery::diagnostics::MAX_REPORT_CHARS + 32);
    }

    #[test]
    fn the_discovery_report_names_an_override_and_keeps_the_detected_answer() {
        let db = Db::open_in_memory().unwrap();
        let id = device_with_detected_type(&db, "media_device", "medium");
        db.set_device_type_override(id, Some("television".into()))
            .unwrap();
        let report = db.device_discovery_report(id, "1.8.3").unwrap();
        assert!(report.contains("Device type: Television"));
        assert!(report.contains("Type source: Set by you"));
        assert!(report.contains("ArcScan detected: Media device"));
    }

    // --- v1.8.3: discovery quality in History ------------------------------

    #[test]
    fn every_discovery_quality_state_reaches_the_history_row() {
        let cases: Vec<(&str, Option<crate::discovery::DiscoveryReport>, &str)> = vec![
            (
                "complete",
                Some(crate::discovery::DiscoveryReport {
                    mdns_attempted: true,
                    ssdp_attempted: true,
                    mdns_responses: 12,
                    ssdp_responses: 8,
                    ..Default::default()
                }),
                "complete",
            ),
            (
                "a socket that would not open",
                Some(crate::discovery::DiscoveryReport {
                    mdns_attempted: true,
                    ssdp_attempted: true,
                    mdns_socket_failed: true,
                    ..Default::default()
                }),
                "limited",
            ),
            (
                "a remote or switched-off scan",
                Some(crate::discovery::DiscoveryReport::skipped("Remote subnet")),
                "skipped",
            ),
            ("a scan with no discovery record at all", None, "skipped"),
        ];
        for (what, report, expected) in cases {
            let db = Db::open_in_memory().unwrap();
            let mut scan = result(
                "10.0.0.0/24",
                None,
                vec![host("10.0.0.5", None, None, &[80])],
            );
            scan.discovery = report;
            db.save_scan(&scan).unwrap();
            let summary = &db.list_scans().unwrap()[0];
            assert_eq!(summary.discovery_quality, expected, "{what}");
            if expected == "complete" {
                assert_eq!(summary.discovery_quality_reason, None, "{what}");
            } else {
                assert!(summary.discovery_quality_reason.is_some(), "{what}");
            }
        }
    }

    #[test]
    fn a_stopped_scan_reads_as_interrupted_in_history() {
        let db = Db::open_in_memory().unwrap();
        let mut scan = result(
            "10.0.0.0/24",
            None,
            vec![host("10.0.0.5", None, None, &[80])],
        );
        scan.discovery = Some(crate::discovery::DiscoveryReport {
            mdns_attempted: true,
            ssdp_attempted: true,
            interrupted: true,
            ..Default::default()
        });
        db.save_scan(&scan).unwrap();
        let summary = &db.list_scans().unwrap()[0];
        assert_eq!(summary.discovery_quality, "interrupted");
        assert_eq!(
            summary.discovery_quality_reason.as_deref(),
            Some("Scan stopped")
        );
        // ArcScan never claims a firewall, because it cannot observe one.
        assert!(!summary
            .discovery_quality_reason
            .as_deref()
            .unwrap()
            .to_lowercase()
            .contains("firewall"));
    }

    #[test]
    fn a_user_name_still_wins_over_a_detected_one() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host(
            "10.0.0.5",
            Some("aa:bb:cc:00:00:05"),
            Some("printer-01"),
            &[631],
        );
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host.clone()],
        )))
        .unwrap();

        let device_id = db.inventory().unwrap().rows[0].device_id;
        db.set_device_name(device_id, Some("Front Office Printer".into()))
            .unwrap();
        // A second scan re-advertises the detected name; the operator's stands.
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();

        let row = &db.inventory().unwrap().rows[0];
        assert_eq!(row.display_name, "Front Office Printer");
        assert_eq!(
            row.discovery.as_ref().unwrap().detected_name.as_deref(),
            Some("Studio Printer"),
            "the detected name is kept, just not shown"
        );
    }

    #[test]
    fn re_observing_the_same_advertisements_does_not_grow_the_evidence_table() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));

        let count = |db: &Db| -> i64 {
            db.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| r.get(0))
                .unwrap()
        };
        let first_seen = |db: &Db| -> String {
            db.lock()
                .unwrap()
                .query_row(
                    "SELECT first_seen FROM discovery_evidence WHERE kind = 'service'",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };

        for _ in 0..5 {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![host.clone()],
            )))
            .unwrap();
        }
        let after_five = count(&db);
        let original_first_seen = first_seen(&db);
        assert!(after_five > 0);

        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();
        assert_eq!(
            count(&db),
            after_five,
            "a re-observation updates, never inserts"
        );
        assert_eq!(
            first_seen(&db),
            original_first_seen,
            "first seen survives every re-observation"
        );
    }

    #[test]
    fn a_settled_high_confidence_type_survives_a_scan_that_heard_nothing_new() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host.clone()],
        )))
        .unwrap();

        // A later scan only manages a weak reading.
        let mut quiet = host.clone();
        quiet.discovery = Some(crate::scanner::HostDiscovery {
            device_type: Some("computer".into()),
            type_confidence: Some("low".into()),
            ..Default::default()
        });
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![quiet],
        )))
        .unwrap();

        let types = discovery_types(&db);
        assert_eq!(types[0].1, "printer");
        assert_eq!(types[0].2, "high");
    }

    #[test]
    fn the_first_sighting_of_a_device_creates_no_discovery_events() {
        // Otherwise the first scan after upgrading would report a detected name
        // and a type for every device on the network at once.
        let db = Db::open_in_memory().unwrap();
        let plain = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![plain.clone()],
        )))
        .unwrap();

        let mut enriched = plain.clone();
        enriched.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![enriched],
        )))
        .unwrap();

        assert!(events_of_type(&db, ChangeType::DetectedNameChanged).is_empty());
        assert!(events_of_type(&db, ChangeType::DeviceTypeChanged).is_empty());
        assert!(events_of_type(&db, ChangeType::ServiceAppeared).is_empty());
    }

    /// Save `count` identical full-discovery scans, returning the database.
    fn seeded_discovery(host: HostResult, count: usize) -> Db {
        let db = Db::open_in_memory().unwrap();
        for _ in 0..count {
            db.save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![host.clone()],
            )))
            .unwrap();
        }
        db
    }

    #[test]
    fn a_meaningful_name_change_is_reported_once_the_device_is_known() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        let mut renamed = host.clone();
        renamed.discovery = Some(discovery_for(
            "Reception Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![renamed],
        )))
        .unwrap();

        let events = events_of_type(&db, ChangeType::DetectedNameChanged);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].old_value.as_deref(), Some("Studio Printer"));
        assert_eq!(events[0].new_value.as_deref(), Some("Reception Printer"));
    }

    #[test]
    fn cosmetic_churn_in_a_name_or_a_model_creates_no_event() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        // Same name and model, differently spaced and cased.
        let mut noisy = host.clone();
        let mut discovery = discovery_for("  STUDIO   printer ", "printer", "high", &["_ipp._tcp"]);
        discovery.model_name = Some("laserfast  400".into());
        discovery.manufacturer = Some("ACME".into());
        noisy.discovery = Some(discovery);
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![noisy],
        )))
        .unwrap();

        assert!(events_of_type(&db, ChangeType::DetectedNameChanged).is_empty());
        assert!(events_of_type(&db, ChangeType::ModelChanged).is_empty());
    }

    #[test]
    fn a_low_confidence_type_change_creates_no_event() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for("Thing", "printer", "low", &[]));
        let db = seeded_discovery(host.clone(), 2);

        let mut changed = host.clone();
        changed.discovery = Some(discovery_for("Thing", "camera", "low", &[]));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![changed],
        )))
        .unwrap();

        assert!(events_of_type(&db, ChangeType::DeviceTypeChanged).is_empty());
    }

    #[test]
    fn one_missed_response_does_not_report_a_service_as_gone() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        // One scan hears nothing from it — ordinary on a lossy link.
        let mut quiet = host.clone();
        quiet.discovery = Some(discovery_for("Studio Printer", "printer", "high", &[]));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![quiet.clone()],
        )))
        .unwrap();
        assert!(
            events_of_type(&db, ChangeType::ServiceDisappeared).is_empty(),
            "one miss is not evidence of removal"
        );

        // A second consecutive miss is.
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![quiet],
        )))
        .unwrap();
        let gone = events_of_type(&db, ChangeType::ServiceDisappeared);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].old_value.as_deref(), Some("_ipp._tcp"));
    }

    #[test]
    fn a_new_service_is_reported_when_it_appears() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        let mut extra = host.clone();
        extra.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp", "_scanner._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![extra],
        )))
        .unwrap();

        let appeared = events_of_type(&db, ChangeType::ServiceAppeared);
        assert_eq!(appeared.len(), 1);
        assert_eq!(appeared[0].new_value.as_deref(), Some("_scanner._tcp"));
    }

    #[test]
    fn a_partial_scan_records_no_discovery_events_at_all() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        let mut renamed = host.clone();
        renamed.discovery = Some(discovery_for(
            "Something Else",
            "camera",
            "high",
            &["_rtsp._tcp"],
        ));
        let mut partial = with_full_discovery(result("10.0.0.0/24", None, vec![renamed]));
        partial.cancelled = true;
        partial.probed = 10;
        db.save_scan(&partial).unwrap();

        for kind in [
            ChangeType::DetectedNameChanged,
            ChangeType::DeviceTypeChanged,
            ChangeType::ServiceAppeared,
            ChangeType::ServiceDisappeared,
            ChangeType::ModelChanged,
        ] {
            assert!(
                events_of_type(&db, kind).is_empty(),
                "a stopped scan produced a {kind:?} event"
            );
        }
    }

    #[test]
    fn a_scan_without_discovery_is_never_compared_against_one_with_it() {
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let db = seeded_discovery(host.clone(), 2);

        // A scan where discovery could not run — a remote target, or the
        // feature switched off. It must not report the services as gone.
        let plain = self::host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        db.save_scan(&result("10.0.0.0/24", None, vec![plain]))
            .unwrap();

        assert!(events_of_type(&db, ChangeType::ServiceDisappeared).is_empty());
        assert!(events_of_type(&db, ChangeType::DetectedNameChanged).is_empty());
    }

    #[test]
    fn the_discovery_mode_is_recorded_with_the_scan() {
        let db = Db::open_in_memory().unwrap();
        let plain = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        db.save_scan(&result("10.0.0.0/24", None, vec![plain.clone()]))
            .unwrap();
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![plain.clone()],
        )))
        .unwrap();
        let mut stopped = with_full_discovery(result("10.0.0.0/24", None, vec![plain]));
        stopped.cancelled = true;
        db.save_scan(&stopped).unwrap();

        let modes: Vec<String> = db
            .list_scans()
            .unwrap()
            .into_iter()
            .map(|s| s.discovery_mode)
            .collect();
        // Newest first: cancelled, full, none.
        assert_eq!(modes, vec!["none", "full", "none"]);
    }

    #[test]
    fn discovery_never_crosses_a_network_scope() {
        let db = Db::open_in_memory().unwrap();
        let mut a = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        a.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));

        let mut office = with_full_discovery(result("10.0.0.0/24", None, vec![a.clone()]));
        office.scope_hint = Some(crate::scanner::ScopeHint {
            local_network: Some("10.0.0.0/24".into()),
            gateway_mac: Some("aa:aa:aa:aa:aa:01".into()),
            ..Default::default()
        });
        db.save_scan(&office).unwrap();

        // The same MAC and the same advertisement, on a different network.
        let mut other = with_full_discovery(result("10.0.0.0/24", None, vec![a]));
        other.scope_hint = Some(crate::scanner::ScopeHint {
            local_network: Some("10.0.0.0/24".into()),
            gateway_mac: Some("bb:bb:bb:bb:bb:02".into()),
            ..Default::default()
        });
        db.save_scan(&other).unwrap();

        let rows = db.inventory().unwrap().rows;
        assert_eq!(rows.len(), 2, "two networks, two devices");
        let scopes: std::collections::HashSet<Option<i64>> =
            rows.iter().map(|r| r.network_scope_id).collect();
        assert_eq!(scopes.len(), 2);
        // Each carries its own discovery record; neither borrowed the other's.
        for row in &rows {
            assert!(row.discovery.is_some());
        }
    }

    #[test]
    fn discovery_does_not_change_how_a_device_is_identified() {
        // The whole identity guarantee: a device keeps its id and its identity
        // key when discovery arrives, and gains no new way of being matched.
        let db = Db::open_in_memory().unwrap();
        let plain = host(
            "10.0.0.5",
            Some("aa:bb:cc:00:00:05"),
            Some("printer-01"),
            &[631],
        );
        db.save_scan(&result("10.0.0.0/24", None, vec![plain.clone()]))
            .unwrap();
        let before = db.list_devices().unwrap();

        let mut enriched = plain.clone();
        enriched.discovery = Some(discovery_for(
            "A Completely Different Name",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![enriched],
        )))
        .unwrap();

        let after = db.list_devices().unwrap();
        assert_eq!(after.len(), before.len(), "no device was created or split");
        assert_eq!(after[0].id, before[0].id);
        assert_eq!(after[0].identity_key, before[0].identity_key);
        assert_eq!(after[0].identity_source, before[0].identity_source);
    }

    #[test]
    fn the_v5_migration_is_idempotent_and_keeps_everything_before_it() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        db.save_scan(&with_full_discovery(result(
            "10.0.0.0/24",
            None,
            vec![host],
        )))
        .unwrap();
        let before = db.inventory().unwrap().rows.len();
        let evidence_before = discovery_types(&db);

        {
            let mut conn = db.lock().unwrap();
            for _ in 0..3 {
                migrate(&mut conn).unwrap();
                migrate_v5(&mut conn).unwrap();
            }
        }

        assert_eq!(db.inventory().unwrap().rows.len(), before);
        assert_eq!(discovery_types(&db), evidence_before);
    }

    #[test]
    fn deleting_a_device_takes_its_discovery_with_it() {
        let db = Db::open_in_memory().unwrap();
        let mut host = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[631]);
        host.discovery = Some(discovery_for(
            "Studio Printer",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        let saved = db
            .save_scan(&with_full_discovery(result(
                "10.0.0.0/24",
                None,
                vec![host],
            )))
            .unwrap();
        assert_eq!(discovery_types(&db).len(), 1);

        db.delete_scan(saved.scan_id).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM devices", []).unwrap();
        }
        assert!(
            discovery_types(&db).is_empty(),
            "no orphaned discovery rows"
        );
        let orphans: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM discovery_evidence", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);
    }

    // --- Disposable Portable database sessions ----------------------------

    /// A temporary directory that removes itself.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "arcscan-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Every table, index and column a database has, as one comparable string.
    fn schema_of(db: &Db) -> Vec<String> {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT type, name, sql FROM sqlite_master
                 WHERE name NOT LIKE 'sqlite_%'
                 ORDER BY type, name",
            )
            .unwrap();
        stmt.query_map([], |row| {
            let kind: String = row.get(0)?;
            let name: String = row.get(1)?;
            let sql: Option<String> = row.get(2)?;
            Ok(format!("{kind} {name}: {}", sql.unwrap_or_default()))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
    }

    fn schema_version(db: &Db) -> i64 {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            .parse()
            .unwrap()
    }

    /// A scan with devices, discovery evidence, a scope and comparison data.
    fn seeded_scan() -> ScanResult {
        let mut nas = host(
            "192.168.1.10",
            Some("AA:BB:CC:00:00:01"),
            Some("nas"),
            &[22, 445],
        );
        nas.discovery = Some(discovery_for("Studio NAS", "nas", "high", &["_smb._tcp"]));
        let mut printer = host(
            "192.168.1.20",
            Some("AA:BB:CC:00:00:02"),
            Some("printer"),
            &[80],
        );
        printer.discovery = Some(discovery_for(
            "LaserFast",
            "printer",
            "high",
            &["_ipp._tcp"],
        ));
        with_full_discovery(result(
            "192.168.1.0/24",
            Some("quick-lan"),
            vec![nas, printer],
        ))
    }

    fn portable_session_db_path(temp: &TempDir, session_id: &str) -> std::path::PathBuf {
        temp.path()
            .join("ArcScanPortable")
            .join("sessions")
            .join(session_id)
            .join("arcscan.db")
    }

    fn device_id_by_mac(db: &Db, mac: &str) -> i64 {
        db.list_devices()
            .unwrap()
            .into_iter()
            .find(|device| device.mac.as_deref() == Some(mac))
            .expect("the seeded device is in this session")
            .id
    }

    #[test]
    fn disposable_installed_and_session_databases_have_identical_schema() {
        let temp = TempDir::new("disposable-schema");
        let installed = Db::open(&temp.path().join("installed").join("arcscan.db")).unwrap();
        let portable = Db::open(&portable_session_db_path(
            &temp,
            "00000000000040008000000000000001",
        ))
        .unwrap();

        assert_eq!(schema_version(&installed), SCHEMA_VERSION);
        assert_eq!(schema_version(&portable), SCHEMA_VERSION);
        assert_eq!(schema_of(&installed), schema_of(&portable));
    }

    #[test]
    fn disposable_session_supports_the_complete_database_workflow() {
        let temp = TempDir::new("disposable-workflow");
        let db = Db::open(&portable_session_db_path(
            &temp,
            "00000000000040008000000000000002",
        ))
        .unwrap();

        let first = db.save_scan(&seeded_scan()).unwrap();
        let nas_id = device_id_by_mac(&db, "AA:BB:CC:00:00:01");
        db.set_device_name(nas_id, Some("Studio NAS".into()))
            .unwrap();
        db.set_device_notes(nas_id, Some("Rack 2, shelf 3".into()))
            .unwrap();
        db.set_device_status(nas_id, DeviceStatus::Trusted).unwrap();
        db.set_device_type_override(nas_id, Some("nas".into()))
            .unwrap();

        let scope_id = db.list_network_scopes().unwrap()[0].id;
        db.rename_network_scope(scope_id, "Studio".into()).unwrap();

        let mut next_scan = seeded_scan();
        next_scan.hosts[0].open_ports.push(443);
        next_scan.hosts.remove(1);
        let second = db.save_scan(&next_scan).unwrap();

        assert_eq!(second.comparison.baseline_scan_id, Some(first.scan_id));
        assert!(
            change_count(&second.comparison) > 0,
            "the second scan should produce comparison changes"
        );

        let history = db.list_scans().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(db.get_scan(first.scan_id).unwrap().hosts.len(), 2);
        assert_eq!(db.get_scan(second.scan_id).unwrap().hosts.len(), 1);

        let inventory = db.inventory().unwrap();
        assert_eq!(inventory.rows.len(), 2);
        let nas = inventory
            .rows
            .iter()
            .find(|row| row.device_id == nas_id)
            .unwrap();
        assert_eq!(nas.custom_name.as_deref(), Some("Studio NAS"));
        assert!(nas.notes_present);
        assert!(nas.discovery.is_some());

        let changes = db.change_events().unwrap();
        assert!(changes.total > 0);
        assert!(!changes.events.is_empty());

        let detail = db.device_detail(nas_id).unwrap();
        assert_eq!(detail.device.custom_name.as_deref(), Some("Studio NAS"));
        assert_eq!(detail.device.notes.as_deref(), Some("Rack 2, shelf 3"));
        assert_eq!(detail.device.status, DeviceStatus::Trusted);
        assert_eq!(detail.device.user_device_type.as_deref(), Some("nas"));
        assert!(detail.discovery.is_some());
        assert_eq!(db.list_network_scopes().unwrap()[0].display_name, "Studio");
    }

    #[test]
    fn disposable_fresh_session_starts_empty() {
        let temp = TempDir::new("disposable-fresh");
        let first = Db::open(&portable_session_db_path(
            &temp,
            "00000000000040008000000000000003",
        ))
        .unwrap();
        first.save_scan(&seeded_scan()).unwrap();
        let device_id = device_id_by_mac(&first, "AA:BB:CC:00:00:01");
        first
            .set_device_name(device_id, Some("Session one".into()))
            .unwrap();
        first
            .set_device_notes(device_id, Some("Discard with session one".into()))
            .unwrap();
        first.shutdown().unwrap();

        let fresh = Db::open(&portable_session_db_path(
            &temp,
            "00000000000040008000000000000004",
        ))
        .unwrap();

        assert_eq!(schema_version(&fresh), SCHEMA_VERSION);
        assert!(fresh.list_scans().unwrap().is_empty());
        assert!(fresh.list_devices().unwrap().is_empty());
        assert!(fresh.inventory().unwrap().rows.is_empty());
        assert!(fresh.change_events().unwrap().events.is_empty());
        assert!(fresh.list_network_scopes().unwrap().is_empty());
        let discovery_rows: i64 = fresh
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM device_discovery", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(discovery_rows, 0);
    }

    #[test]
    fn disposable_simultaneous_sessions_are_isolated() {
        let temp = TempDir::new("disposable-concurrent");
        let path_a = portable_session_db_path(&temp, "00000000000040008000000000000005");
        let path_b = portable_session_db_path(&temp, "00000000000040008000000000000006");
        let session_a = Db::open(&path_a).unwrap();
        let session_b = Db::open(&path_b).unwrap();

        session_a.save_scan(&seeded_scan()).unwrap();
        let device_a = device_id_by_mac(&session_a, "AA:BB:CC:00:00:01");
        session_a
            .set_device_name(device_a, Some("Session A NAS".into()))
            .unwrap();

        assert!(session_b.list_scans().unwrap().is_empty());
        assert!(session_b.list_devices().unwrap().is_empty());

        session_b.save_scan(&seeded_scan()).unwrap();
        let device_b = device_id_by_mac(&session_b, "AA:BB:CC:00:00:01");
        session_b
            .set_device_name(device_b, Some("Session B NAS".into()))
            .unwrap();

        assert_ne!(path_a, path_b);
        assert_eq!(session_a.list_scans().unwrap().len(), 1);
        assert_eq!(session_b.list_scans().unwrap().len(), 1);
        assert_eq!(
            session_a
                .device_detail(device_a)
                .unwrap()
                .device
                .custom_name
                .as_deref(),
            Some("Session A NAS")
        );
        assert_eq!(
            session_b
                .device_detail(device_b)
                .unwrap()
                .device
                .custom_name
                .as_deref(),
            Some("Session B NAS")
        );
    }

    #[test]
    fn disposable_installed_and_portable_databases_are_isolated() {
        let temp = TempDir::new("disposable-editions");
        let installed = Db::open(&temp.path().join("installed").join("arcscan.db")).unwrap();
        let portable = Db::open(&portable_session_db_path(
            &temp,
            "00000000000040008000000000000007",
        ))
        .unwrap();

        installed.save_scan(&seeded_scan()).unwrap();
        let installed_device = device_id_by_mac(&installed, "AA:BB:CC:00:00:01");
        installed
            .set_device_name(installed_device, Some("Installed NAS".into()))
            .unwrap();
        installed
            .set_device_notes(installed_device, Some("Installed only".into()))
            .unwrap();

        assert!(portable.list_scans().unwrap().is_empty());
        assert!(portable.list_devices().unwrap().is_empty());

        portable.save_scan(&seeded_scan()).unwrap();
        let portable_device = device_id_by_mac(&portable, "AA:BB:CC:00:00:01");
        portable
            .set_device_name(portable_device, Some("Portable NAS".into()))
            .unwrap();
        portable
            .set_device_notes(portable_device, Some("Portable session only".into()))
            .unwrap();

        let installed_detail = installed.device_detail(installed_device).unwrap();
        let portable_detail = portable.device_detail(portable_device).unwrap();
        assert_eq!(
            installed_detail.device.custom_name.as_deref(),
            Some("Installed NAS")
        );
        assert_eq!(
            installed_detail.device.notes.as_deref(),
            Some("Installed only")
        );
        assert_eq!(
            portable_detail.device.custom_name.as_deref(),
            Some("Portable NAS")
        );
        assert_eq!(
            portable_detail.device.notes.as_deref(),
            Some("Portable session only")
        );
    }

    #[test]
    fn disposable_shutdown_is_idempotent_and_releases_the_session_database() {
        let temp = TempDir::new("disposable-shutdown");
        let db_path = portable_session_db_path(&temp, "00000000000040008000000000000008");
        let session_root = db_path.parent().unwrap().to_path_buf();
        let db = Db::open(&db_path).unwrap();
        db.save_scan(&seeded_scan()).unwrap();

        db.shutdown().unwrap();
        db.shutdown().unwrap();

        assert!(
            db.lock().is_err(),
            "shutdown must remove the connection handle"
        );
        let read_error = db
            .list_scans()
            .expect_err("database reads after shutdown must fail");
        let write_error = db
            .save_scan(&seeded_scan())
            .expect_err("database writes after shutdown must fail");
        assert!(read_error.contains("closed"), "{read_error}");
        assert!(write_error.contains("closed"), "{write_error}");

        std::fs::remove_dir_all(&session_root).unwrap();
        assert!(
            !session_root.exists(),
            "SQLite must release the session database and sidecar files"
        );
    }
}
