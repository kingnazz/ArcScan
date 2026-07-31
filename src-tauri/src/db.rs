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
    self, ChangeKind, Device, DeviceStatus, IdentifiedHost, IdentitySource, ScanComparison,
};
use crate::ipparse;
use crate::scanner::{HostResult, ScanResult};
use crate::signature;

/// Current schema version. Bump when a migration is added below.
const SCHEMA_VERSION: i64 = 3;

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

    pub fn device_detail(&self, id: i64) -> Result<DeviceDetail, String> {
        let conn = self.lock()?;
        let device = conn
            .query_row(
                "SELECT d.id, d.network_scope_id, d.identity_key, d.identity_source, d.mac,
                        d.custom_name, d.hostname, d.vendor, d.last_ip, d.first_seen,
                        d.last_seen, d.status, d.notes,
                        (SELECT COUNT(*) FROM hosts h WHERE h.device_id = d.id)
                 FROM devices d WHERE d.id = ?1",
                params![id],
                read_device,
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

        Ok(DeviceDetail {
            device,
            observations,
            previous_ips,
            recent_changes,
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

    // Oldest first, so which scope adopts a newly learned gateway or gets
    // reused without evidence is deterministic.
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

    if version < 2 {
        backfill_v2(conn)?;
    }
    if version < 3 {
        migrate_v3(conn)?;
    }

    // Indexes last: the scope-aware ones only exist once the v3 shape does.
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen DESC);
        CREATE INDEX IF NOT EXISTS idx_devices_scope_mac ON devices(network_scope_id, mac);
        CREATE INDEX IF NOT EXISTS idx_hosts_device      ON hosts(device_id);
        CREATE INDEX IF NOT EXISTS idx_hosts_scan_ip     ON hosts(scan_id, ip);
        CREATE INDEX IF NOT EXISTS idx_scans_target_key  ON scans(target_key, id DESC);
        CREATE INDEX IF NOT EXISTS idx_scans_baseline
            ON scans(network_scope_id, target_key, coverage_key, status, id DESC);
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
}
