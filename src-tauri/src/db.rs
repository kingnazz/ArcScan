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
use std::path::Path;
use std::sync::Mutex;

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
const SCHEMA_VERSION: i64 = 4;

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
    conn: Mutex<Connection>,
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
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database. Used by the migration and inventory tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let mut conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "The scan database is unavailable because a previous write failed.".into())
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
        tx.execute(
            "INSERT INTO scans
                (target, target_key, profile, created_at, duration_ms, scanned, probed, status,
                 network_scope_id, coverage_key, execution_settings)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            ],
        )
        .map_err(sql_err)?;
        let scan_id = tx.last_insert_rowid();

        // Hosts found before a Stop are genuine observations, so they fold into
        // the inventory either way.
        let mut current: Vec<IdentifiedHost> = Vec::with_capacity(result.hosts.len());
        for host in &result.hosts {
            let record = upsert_device(&tx, scope_id, host, &created_at)?;
            insert_observation(&tx, scan_id, record.id, host)?;
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
                        s.network_scope_id, ns.display_name, s.coverage_key
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
                        s.network_scope_id, ns.display_name, s.coverage_key
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
        let conn = self.lock()?;
        conn.execute("DELETE FROM scans WHERE id = ?1", params![id])
            .map_err(sql_err)?;
        Ok(())
    }

    /// Drop the oldest scans, keeping the newest `keep`. Devices survive so
    /// labels, notes and first-seen dates are never lost to retention.
    pub fn prune_history(&self, keep: i64) -> Result<usize, String> {
        if keep < 1 {
            return Err("History retention must keep at least one scan.".into());
        }
        let conn = self.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM scans WHERE id NOT IN
                    (SELECT id FROM scans ORDER BY id DESC LIMIT ?1)",
                params![keep],
            )
            .map_err(sql_err)?;
        Ok(removed)
    }

    /// The whole device inventory, newest sighting first.
    pub fn list_devices(&self) -> Result<Vec<Device>, String> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.network_scope_id, d.identity_key, d.identity_source, d.mac,
                        d.custom_name, d.hostname, d.vendor, d.last_ip, d.first_seen,
                        d.last_seen, d.status, d.notes, COUNT(h.id)
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
                Ok(InventoryRow {
                    device_id,
                    network_scope_id: row.get(1)?,
                    network_name: row.get(2)?,
                    identity_source: parse_source(&row.get::<_, String>(3)?),
                    display_name: inventory::display_name(
                        custom_name.as_deref(),
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
        networks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

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
                        ns.display_name
                 FROM devices d
                 LEFT JOIN network_scopes ns ON ns.id = d.network_scope_id
                 WHERE d.id = ?1",
                params![id],
                |row| Ok((read_device(row)?, row.get::<_, Option<String>>(14)?)),
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

        Ok(DeviceDetail {
            device,
            observations,
            previous_ips,
            recent_changes,
            events,
            network_name,
            presence,
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
       SUBSTR(d.notes, 1, 160) AS notes_excerpt
FROM devices d
LEFT JOIN network_scopes ns ON ns.id = d.network_scope_id
LEFT JOIN reference r ON r.scope_id IS d.network_scope_id
LEFT JOIN latest l ON l.device_id = d.id
LEFT JOIN counts c ON c.device_id = d.id
LEFT JOIN present p ON p.device_id = d.id
LEFT JOIN comparable cmp ON cmp.device_id = d.id
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
    let sql = "SELECT id, target, created_at FROM scans
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
    })
}

fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanSummary> {
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

    if version < 2 {
        backfill_v2(conn)?;
    }
    if version < 3 {
        migrate_v3(conn)?;
    }
    if version < 4 {
        migrate_v4(conn)?;
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
        }
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
                        Some("aa:bb:cc:00:00:01"),
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
            vec![host("10.0.0.1", Some("aa:bb:cc:00:00:01"), None, &[])],
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
                    Some("aa:bb:cc:00:00:01"),
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
                    Some("aa:bb:cc:00:00:01"),
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
                Some("aa:bb:cc:00:00:01"),
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
                Some("aa:bb:cc:00:00:01"),
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
                    Some("aa:bb:cc:00:00:01"),
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
                Some("aa:bb:cc:00:00:01"),
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
            vec![host("10.0.0.1", Some("aa:bb:cc:00:00:01"), None, &[80])],
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
                    Some("aa:bb:cc:00:00:01"),
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
                Some("aa:bb:cc:00:00:01"),
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
        let gateway = host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]);
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
                Some("aa:bb:cc:00:00:01"),
                Some("gw"),
                &[80],
            )],
        );
        db.save_scan(&first).unwrap();
        let second = result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
                host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("tv"), &[8009]),
            ],
        );
        let saved = db.save_scan(&second).unwrap();
        let before = db.change_events().unwrap().total;
        assert_eq!(before, 1);

        // Re-record the identical comparison, the way a retried save would.
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            let baseline = BaselineScan {
                id: saved.comparison.baseline_scan_id.unwrap(),
                target: "10.0.0.0/24".into(),
                created_at: saved.comparison.baseline_created_at.clone().unwrap(),
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
                Some("aa:bb:cc:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
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
                Some("aa:bb:cc:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
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
        let gateway = host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]);
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
                Some("aa:bb:cc:00:00:01"),
                Some("gw"),
                &[80],
            )],
        ))
        .unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![
                host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
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
                Some("aa:bb:cc:00:00:01"),
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
                    host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
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

        // The inventory is intact too.
        assert_eq!(db.inventory().unwrap().rows.len(), 2);
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
                host("10.0.0.1", Some("aa:bb:cc:00:00:01"), Some("gw"), &[80]),
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
                Some("aa:bb:cc:00:00:01"),
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
}
