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
//! # Migrations
//!
//! Every migration is idempotent: `schema_meta` records the version reached,
//! `ALTER TABLE ... ADD COLUMN` failures for already-present columns are
//! ignored (SQLite has no `ADD COLUMN IF NOT EXISTS`), and the backfill only
//! touches rows whose `device_id` is still NULL. Opening a database repeatedly,
//! or opening a v1.7 database with a v1.7 build, changes nothing.

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

/// Current schema version. Bump when a migration is added below.
const SCHEMA_VERSION: i64 = 2;

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
    pub fn save_scan(&self, result: &ScanResult) -> Result<SavedScan, String> {
        let mut conn = self.lock()?;
        let created_at = chrono::Local::now().to_rfc3339();
        // A target that reached the scanner always parses, so a failure here can
        // only mean a hand-crafted call; fall back to the raw string rather than
        // refusing to save real results.
        let target_key =
            ipparse::canonical_key(&result.target).unwrap_or_else(|_| result.target.clone());
        let tx = conn.transaction().map_err(sql_err)?;

        // Pick the baseline before inserting, so the new scan cannot be its own
        // comparison point.
        let baseline = find_baseline(&tx, &target_key, result.profile.as_deref(), None)?;
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
                (target, target_key, profile, created_at, duration_ms, scanned, probed, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                result.target,
                target_key,
                result.profile,
                created_at,
                result.duration_ms as i64,
                result.scanned as i64,
                result.probed as i64,
                status,
            ],
        )
        .map_err(sql_err)?;
        let scan_id = tx.last_insert_rowid();

        let mut current: Vec<IdentifiedHost> = Vec::with_capacity(result.hosts.len());
        for host in &result.hosts {
            let record = upsert_device(&tx, host, &created_at)?;
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
        let comparison = match &baseline {
            None => ScanComparison::empty(
                scan_id,
                "This is the first scan of this target and profile, so there is nothing to \
                 compare it with yet.",
            ),
            Some(b) => {
                let mut c = inventory::compare(scan_id, &baseline_hosts, &current);
                c.baseline_scan_id = Some(b.id);
                c.baseline_created_at = Some(b.created_at.clone());
                c.baseline_target = Some(b.target.clone());
                c
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
                        s.changed_count, s.status, s.baseline_scan_id
                 FROM scans s LEFT JOIN hosts h ON h.scan_id = s.id
                 GROUP BY s.id
                 ORDER BY s.id DESC",
            )
            .map_err(sql_err)?;
        let rows = stmt.query_map([], read_summary).map_err(sql_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_err)
    }

    pub fn get_scan(&self, id: i64) -> Result<ScanDetail, String> {
        let conn = self.lock()?;
        let summary = conn
            .query_row(
                "SELECT s.id, s.target, s.target_key, s.profile, s.created_at, s.duration_ms,
                        s.scanned, s.probed, COUNT(h.id), s.new_count, s.missing_count,
                        s.changed_count, s.status, s.baseline_scan_id
                 FROM scans s LEFT JOIN hosts h ON h.scan_id = s.id
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
    /// it. Compatibility means the same normalized target *and* the same profile:
    /// a Quick LAN sweep and a Full TCP sweep of one subnet see different
    /// services, so diffing them would report invented port changes.
    pub fn compare_scan(&self, id: i64) -> Result<ScanComparison, String> {
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction().map_err(sql_err)?;
        let Some((target_key, profile)) = tx
            .query_row(
                "SELECT target_key, profile FROM scans WHERE id = ?1",
                params![id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(sql_err)?
        else {
            return Err(format!("Scan {id} is no longer in the history."));
        };

        let baseline = find_baseline(&tx, &target_key, profile.as_deref(), Some(id))?;
        let Some(baseline) = baseline else {
            return Ok(ScanComparison::empty(
                id,
                "No earlier scan of this target and profile exists, so there is nothing to \
                 compare against.",
            ));
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
                "SELECT d.id, d.identity_key, d.identity_source, d.mac, d.custom_name, d.hostname,
                        d.vendor, d.last_ip, d.first_seen, d.last_seen, d.status, d.notes,
                        COUNT(h.id)
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
                "SELECT d.id, d.identity_key, d.identity_source, d.mac, d.custom_name, d.hostname,
                        d.vendor, d.last_ip, d.first_seen, d.last_seen, d.status, d.notes,
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

/// The most recent scan that covers the same normalized target with the same
/// profile. `before` excludes the scan being compared and everything after it.
fn find_baseline(
    tx: &Transaction<'_>,
    target_key: &str,
    profile: Option<&str>,
    before: Option<i64>,
) -> Result<Option<BaselineScan>, String> {
    // COALESCE keeps NULL profiles matching each other, which SQL equality does
    // not do; a scan with no profile still compares against earlier scans with
    // no profile.
    let sql = "SELECT id, target, created_at FROM scans
               WHERE target_key = ?1
                 AND COALESCE(profile, '') = COALESCE(?2, '')
                 AND id < ?3
               ORDER BY id DESC LIMIT 1";
    tx.query_row(
        sql,
        params![target_key, profile, before.unwrap_or(i64::MAX)],
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

/// Find or create the device for one observation.
fn upsert_device(
    tx: &Transaction<'_>,
    host: &HostResult,
    seen_at: &str,
) -> Result<DeviceRecord, String> {
    let identity = inventory::identify(host);

    // Look up by MAC first: a device seen earlier without one (routed scan, or a
    // scan where ARP had not resolved yet) was stored under a hostname or IP key,
    // and must be recognised rather than duplicated.
    let existing = if let Some(mac) = &identity.mac {
        tx.query_row(
            "SELECT id, identity_key, custom_name FROM devices WHERE mac = ?1",
            params![mac],
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
                "SELECT id, identity_key, custom_name FROM devices WHERE identity_key = ?1",
                params![identity.key],
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

    // A MAC-identified observation can also claim a MAC-less device that matches
    // on hostname and vendor, or on the same address. This is the common case of
    // ARP resolving on a later scan, and adopting the old row keeps the device's
    // first-seen date, name and notes.
    let existing = match (existing, &identity.mac) {
        (None, Some(_)) => {
            let fallback_key = {
                let mut probe = host.clone();
                probe.mac = None;
                inventory::identify(&probe).key
            };
            tx.query_row(
                "SELECT id, identity_key, custom_name FROM devices
                 WHERE mac IS NULL AND (identity_key = ?1 OR (identity_key = ?2))
                 ORDER BY id ASC LIMIT 1",
                params![fallback_key, format!("ip:{}", host.ip)],
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
            (identity_key, identity_source, mac, hostname, vendor, last_ip,
             first_seen, last_seen, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 'unclassified')",
        params![
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
    })
}

fn read_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        identity_key: row.get(1)?,
        identity_source: parse_source(&row.get::<_, String>(2)?),
        mac: row.get(3)?,
        custom_name: row.get(4)?,
        hostname: row.get(5)?,
        vendor: row.get(6)?,
        last_ip: row.get(7)?,
        first_seen: row.get(8)?,
        last_seen: row.get(9)?,
        status: DeviceStatus::parse(&row.get::<_, String>(10)?),
        notes: row.get(11)?,
        observation_count: row.get(12)?,
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
    ] {
        let _ = conn.execute(stmt, []);
    }

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS devices (
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
        CREATE INDEX IF NOT EXISTS idx_devices_mac       ON devices(mac);
        CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen DESC);
        CREATE INDEX IF NOT EXISTS idx_hosts_device      ON hosts(device_id);
        CREATE INDEX IF NOT EXISTS idx_hosts_scan_ip     ON hosts(scan_id, ip);
        CREATE INDEX IF NOT EXISTS idx_scans_target_key  ON scans(target_key, id DESC);
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

    conn.execute(
        "INSERT INTO schema_meta (key, value) VALUES ('version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION.to_string()],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Backfill the v1.7 columns from existing v1.6 rows: normalize every scan's
/// target into a comparison key, and build the device inventory from the
/// observations already on disk so history opens with names and first-seen dates
/// instead of an empty inventory.
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

    // Observations -> devices, oldest scan first so first_seen is truthful.
    let rows: Vec<(i64, HostResult, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT h.id, h.ip, h.hostname, h.mac, h.vendor, h.open_ports, h.response_ms,
                        h.icmp_ms, h.tcp_ms, h.ttl, h.os_guess, h.last_seen, s.created_at
                 FROM hosts h JOIN scans s ON s.id = h.scan_id
                 WHERE h.device_id IS NULL
                 ORDER BY h.scan_id ASC, h.id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                let host_id: i64 = row.get(0)?;
                let host = HostResult {
                    ip: row.get(1)?,
                    hostname: row.get(2)?,
                    mac: row.get(3)?,
                    vendor: row.get(4)?,
                    open_ports: parse_ports(&row.get::<_, String>(5)?),
                    response_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                    icmp_ms: row.get(7)?,
                    tcp_ms: row.get(8)?,
                    ttl: row.get::<_, Option<i64>>(9)?.map(|v| v as u8),
                    os_guess: row.get(10)?,
                    last_seen: row.get(11)?,
                };
                let scan_created: String = row.get(12)?;
                Ok((host_id, host, scan_created))
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };

    for (host_id, host, seen_at) in rows {
        let record = upsert_device(&tx, &host, &seen_at)?;
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
        }
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
    fn comparison_requires_a_compatible_target_and_profile() {
        let db = Db::open_in_memory().unwrap();
        db.save_scan(&result(
            "10.0.0.0/24",
            Some("quick-lan"),
            vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
        ))
        .unwrap();

        // Different network: not a baseline.
        let other_net = db
            .save_scan(&result(
                "192.168.5.0/24",
                Some("quick-lan"),
                vec![host("192.168.5.5", Some("aa:bb:cc:00:00:15"), None, &[])],
            ))
            .unwrap();
        assert!(other_net.comparison.baseline_scan_id.is_none());

        // Same network, different profile: not a baseline either, because the
        // port sets differ and every service would look like a change.
        let other_profile = db
            .save_scan(&result(
                "10.0.0.0/24",
                Some("full-tcp"),
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(other_profile.comparison.baseline_scan_id.is_none());

        // Same network and profile, written differently: this one matches.
        let same = db
            .save_scan(&result(
                "10.0.0.77/24",
                Some("quick-lan"),
                vec![host("10.0.0.5", Some("aa:bb:cc:00:00:05"), None, &[])],
            ))
            .unwrap();
        assert!(same.comparison.baseline_scan_id.is_some());
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

        // Comparison works on the migrated history.
        let cmp = db.compare_scan(scans[0].id).unwrap();
        assert_eq!(cmp.baseline_scan_id, Some(scans[1].id));
        assert_eq!(cmp.changed.len(), 1);
        let fields = &cmp.changed[0].fields;
        assert!(fields.iter().any(|f| f.field == "ip"));
        assert!(fields
            .iter()
            .any(|f| f.field == "ports" && f.added_ports == vec![9100]));

        drop(db);

        // Re-opening runs every migration again and must change nothing.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_scans().unwrap().len(), 2);
        assert_eq!(db.list_devices().unwrap().len(), 2);
        // A third open, to prove idempotency is not a one-shot.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_devices().unwrap().len(), 2);
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
