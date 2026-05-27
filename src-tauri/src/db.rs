//! SQLite persistence for scan history and discovered hosts.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::scanner::{Host, PortResult, ScanResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub id: i64,
    pub target: String,
    pub started_at: String,
    pub finished_at: String,
    pub hosts_up: i64,
    pub total_scanned: i64,
}

pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create data dir: {e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("Failed to open database: {e}"))?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS scans (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            target        TEXT NOT NULL,
            started_at    TEXT NOT NULL,
            finished_at   TEXT NOT NULL,
            total_scanned INTEGER NOT NULL DEFAULT 0,
            hosts_up      INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS hosts (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            scan_id     INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            ip          TEXT NOT NULL,
            hostname    TEXT,
            mac         TEXT,
            vendor      TEXT,
            open_ports  TEXT NOT NULL DEFAULT '[]',
            response_ms INTEGER,
            status      TEXT NOT NULL,
            last_seen   TEXT NOT NULL,
            is_new      INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_hosts_scan ON hosts(scan_id);
        "#,
    )
    .map_err(|e| format!("Failed to initialize schema: {e}"))
}

/// Persist a finished scan and all of its hosts in a single transaction.
/// Returns the new scan id.
pub fn save_scan(conn: &mut Connection, result: &ScanResult) -> Result<i64, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to begin transaction: {e}"))?;

    let hosts_up = result.hosts.iter().filter(|h| h.status == "up").count() as i64;

    tx.execute(
        "INSERT INTO scans (target, started_at, finished_at, total_scanned, hosts_up)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            result.target,
            result.started_at,
            result.finished_at,
            result.total_scanned as i64,
            hosts_up,
        ],
    )
    .map_err(|e| format!("Failed to save scan: {e}"))?;

    let scan_id = tx.last_insert_rowid();

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO hosts
                    (scan_id, ip, hostname, mac, vendor, open_ports, response_ms, status, last_seen, is_new)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(|e| format!("Failed to prepare host insert: {e}"))?;

        for host in &result.hosts {
            let open_ports = serde_json::to_string(&host.open_ports).unwrap_or_else(|_| "[]".into());
            stmt.execute(params![
                scan_id,
                host.ip,
                host.hostname,
                host.mac,
                host.vendor,
                open_ports,
                host.response_ms.map(|v| v as i64),
                host.status,
                host.last_seen,
                host.is_new as i64,
            ])
            .map_err(|e| format!("Failed to save host: {e}"))?;
        }
    }

    tx.commit().map_err(|e| format!("Failed to commit scan: {e}"))?;
    Ok(scan_id)
}

pub fn list_scans(conn: &Connection) -> Result<Vec<ScanSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, target, started_at, finished_at, hosts_up, total_scanned
             FROM scans ORDER BY id DESC LIMIT 200",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ScanSummary {
                id: row.get(0)?,
                target: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                hosts_up: row.get(4)?,
                total_scanned: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_scan_hosts(conn: &Connection, scan_id: i64) -> Result<Vec<Host>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT ip, hostname, mac, vendor, open_ports, response_ms, status, last_seen, is_new
             FROM hosts WHERE scan_id = ?1 ORDER BY id",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![scan_id], |row| {
            let open_ports_json: String = row.get(4)?;
            let open_ports: Vec<PortResult> =
                serde_json::from_str(&open_ports_json).unwrap_or_default();
            let response_ms: Option<i64> = row.get(5)?;
            Ok(Host {
                ip: row.get(0)?,
                hostname: row.get(1)?,
                mac: row.get(2)?,
                vendor: row.get(3)?,
                open_ports,
                response_ms: response_ms.map(|v| v as u64),
                status: row.get(6)?,
                last_seen: row.get(7)?,
                is_new: row.get::<_, i64>(8)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn delete_scan(conn: &Connection, scan_id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM scans WHERE id = ?1", params![scan_id])
        .map_err(|e| format!("Failed to delete scan: {e}"))?;
    Ok(())
}

/// IPs of the most recent saved scan, used to flag newly-appeared devices.
pub fn last_scan_ips(conn: &Connection) -> Result<HashSet<String>, String> {
    let last_id: Option<i64> = conn
        .query_row("SELECT id FROM scans ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .ok();

    let Some(id) = last_id else {
        return Ok(HashSet::new());
    };

    let mut stmt = conn
        .prepare("SELECT ip FROM hosts WHERE scan_id = ?1")
        .map_err(|e| e.to_string())?;
    let ips = stmt
        .query_map(params![id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ips)
}
