//! SQLite persistence for scan history (bundled rusqlite — no system SQLite).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::scanner::{HostResult, ScanResult};

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub id: i64,
    pub target: String,
    pub created_at: String,
    pub duration_ms: i64,
    pub scanned: i64,
    pub host_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDetail {
    #[serde(flatten)]
    pub summary: ScanSummary,
    pub hosts: Vec<HostResult>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
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
        .map_err(|e| e.to_string())?;

        // Idempotent migrations for databases created by earlier versions that
        // predate the ttl/os_guess columns. SQLite has no "ADD COLUMN IF NOT
        // EXISTS", so we run them and ignore the "duplicate column" error.
        for stmt in [
            "ALTER TABLE hosts ADD COLUMN ttl INTEGER",
            "ALTER TABLE hosts ADD COLUMN os_guess TEXT",
        ] {
            let _ = conn.execute(stmt, []);
        }

        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    pub fn save_scan(&self, result: &ScanResult) -> Result<i64, String> {
        let mut conn = self.conn.lock().map_err(|_| "db lock poisoned")?;
        let created_at = chrono::Local::now().to_rfc3339();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO scans (target, created_at, duration_ms, scanned) VALUES (?1, ?2, ?3, ?4)",
            params![
                result.target,
                created_at,
                result.duration_ms as i64,
                result.scanned as i64
            ],
        )
        .map_err(|e| e.to_string())?;
        let scan_id = tx.last_insert_rowid();
        for h in &result.hosts {
            let ports = h
                .open_ports
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                "INSERT INTO hosts (scan_id, ip, hostname, mac, vendor, open_ports, response_ms, ttl, os_guess, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    scan_id,
                    h.ip,
                    h.hostname,
                    h.mac,
                    h.vendor,
                    ports,
                    h.response_ms.map(|v| v as i64),
                    h.ttl.map(|v| v as i64),
                    h.os_guess,
                    h.last_seen,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(scan_id)
    }

    pub fn list_scans(&self) -> Result<Vec<ScanSummary>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.target, s.created_at, s.duration_ms, s.scanned,
                        (SELECT COUNT(*) FROM hosts h WHERE h.scan_id = s.id) AS host_count
                 FROM scans s ORDER BY s.id DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ScanSummary {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    created_at: row.get(2)?,
                    duration_ms: row.get(3)?,
                    scanned: row.get(4)?,
                    host_count: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_scan(&self, id: i64) -> Result<ScanDetail, String> {
        let conn = self.conn.lock().map_err(|_| "db lock poisoned")?;
        let summary = conn
            .query_row(
                "SELECT s.id, s.target, s.created_at, s.duration_ms, s.scanned,
                        (SELECT COUNT(*) FROM hosts h WHERE h.scan_id = s.id) AS host_count
                 FROM scans s WHERE s.id = ?1",
                params![id],
                |row| {
                    Ok(ScanSummary {
                        id: row.get(0)?,
                        target: row.get(1)?,
                        created_at: row.get(2)?,
                        duration_ms: row.get(3)?,
                        scanned: row.get(4)?,
                        host_count: row.get(5)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT ip, hostname, mac, vendor, open_ports, response_ms, ttl, os_guess, last_seen
                 FROM hosts WHERE scan_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let hosts = stmt
            .query_map(params![id], |row| {
                let ports: String = row.get(4)?;
                let open_ports = ports
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse::<u16>().ok())
                    .collect::<Vec<_>>();
                Ok(HostResult {
                    ip: row.get(0)?,
                    hostname: row.get(1)?,
                    mac: row.get(2)?,
                    vendor: row.get(3)?,
                    open_ports,
                    response_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                    ttl: row.get::<_, Option<i64>>(6)?.map(|v| v as u8),
                    os_guess: row.get(7)?,
                    last_seen: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(ScanDetail { summary, hosts })
    }

    pub fn delete_scan(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock poisoned")?;
        conn.execute("DELETE FROM scans WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Return the host IPs of the most recent saved scan, used to compute
    /// "new devices since last scan" on the dashboard.
    pub fn last_scan_ips(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock poisoned")?;
        let last_id: Option<i64> = conn
            .query_row("SELECT id FROM scans ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .ok();
        let Some(last_id) = last_id else {
            return Ok(Vec::new());
        };
        let mut stmt = conn
            .prepare("SELECT ip FROM hosts WHERE scan_id = ?1")
            .map_err(|e| e.to_string())?;
        let ips = stmt
            .query_map(params![last_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(ips)
    }
}
