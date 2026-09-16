//! Orchestrate topology discovery: credentials, bounded concurrency, per-device
//! isolation, cancellation, a wall-clock budget.
//!
//! This crate does not depend on the ArcScan scanner. The host process can
//! register a scan-cancel hook so Stop on an in-flight scan also stops topology.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::stream::{self, StreamExt};

use super::collect::DeviceView;
use super::correlate::correlate;
use super::credentials::{CredentialStore, SnmpSecret};
use super::error::TopologyError;
use super::model::{
    TopologyConfidence, TopologyDeviceFailure, TopologyRequest, TopologyResult, TopologySummary,
    TopologyTarget,
};
use super::providers::{SnmpProvider, TopologyProvider};
use super::snmp::open_session;

const DEFAULT_TIMEOUT_MS: u64 = 800;
const DEFAULT_CONCURRENCY: usize = 8;
const MAX_CONCURRENCY: usize = 16;
const MAX_TARGETS: usize = 512;
const MAX_TOTAL_MS: u64 = 45_000;
const COLLECT_TIMEOUT_MS: u64 = 8_000;

static ACTIVE_TOPOLOGY: AtomicU64 = AtomicU64::new(0);
static CANCEL_TOPOLOGY: AtomicU64 = AtomicU64::new(0);
static NEXT_TOPOLOGY_ID: AtomicU64 = AtomicU64::new(1);
static SCAN_CANCEL_CHECK: OnceLock<fn(u64) -> bool> = OnceLock::new();

/// Ask the in-flight topology run to stop. Safe to call when none is running.
pub fn request_cancel() {
    CANCEL_TOPOLOGY.store(ACTIVE_TOPOLOGY.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Optional hook so the host process can treat scanner Stop as topology Stop.
/// Set once at process start. Tests leave it unset.
pub fn set_scan_cancel_check(check: fn(u64) -> bool) {
    let _ = SCAN_CANCEL_CHECK.set(check);
}

fn cancelled(id: u64, scan_id: Option<u64>) -> bool {
    (id != 0 && CANCEL_TOPOLOGY.load(Ordering::Relaxed) == id)
        || scan_id.is_some_and(|sid| SCAN_CANCEL_CHECK.get().is_some_and(|f| f(sid)))
}

enum DeviceOutcome {
    View(Box<DeviceView>),
    Failed { ip: String, reason: String },
    Cancelled,
}

pub type SessionFactory = Arc<
    dyn Fn(Ipv4Addr, &SnmpSecret) -> Result<Box<dyn super::snmp::SnmpSession>, TopologyError>
        + Send
        + Sync,
>;

async fn probe_one(
    provider: &SnmpProvider,
    secret: &SnmpSecret,
    target: TopologyTarget,
    timeout: Duration,
    factory: Option<SessionFactory>,
) -> DeviceOutcome {
    let ip: Ipv4Addr = match target.ip.parse() {
        Ok(ip) => ip,
        Err(_) => {
            return DeviceOutcome::Failed {
                ip: target.ip,
                reason: "Address is not a valid IPv4 address.".into(),
            };
        }
    };
    let collect = async {
        let session = if let Some(factory) = &factory {
            factory(ip, secret)?
        } else {
            open_session(ip, secret, timeout).await?
        };
        match tokio::time::timeout(
            Duration::from_millis(COLLECT_TIMEOUT_MS),
            provider.collect(session.as_ref(), ip, target.device_id),
        )
        .await
        {
            Ok(Ok(view)) => Ok(view),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(TopologyError::Timeout),
        }
    };
    match collect.await {
        Ok(view) => DeviceOutcome::View(Box::new(view)),
        Err(err) => DeviceOutcome::Failed {
            ip: ip.to_string(),
            reason: super::credentials::sanitize_text(err.message(), Some(secret)),
        },
    }
}

/// Run topology discovery. `inventory` is the full scan/inventory index so
/// endpoints that never spoke SNMP can still be attached via FDB/LLDP.
pub async fn run(
    secret: &SnmpSecret,
    inventory: Vec<TopologyTarget>,
    snmp_targets: Vec<TopologyTarget>,
    timeout: Duration,
    concurrency: usize,
    scan_id: Option<u64>,
    factory: Option<SessionFactory>,
) -> TopologyResult {
    let topology_id = NEXT_TOPOLOGY_ID.fetch_add(1, Ordering::Relaxed);
    ACTIVE_TOPOLOGY.store(topology_id, Ordering::Relaxed);
    CANCEL_TOPOLOGY.store(0, Ordering::Relaxed);

    let started = Instant::now();
    let captured_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let concurrency = concurrency.clamp(1, MAX_CONCURRENCY);
    let stop = Arc::new(AtomicBool::new(false));
    let provider = SnmpProvider;
    let deadline = started + Duration::from_millis(MAX_TOTAL_MS);

    let mut work = snmp_targets;
    work.retain(|t| t.ip.parse::<Ipv4Addr>().is_ok());
    work.truncate(MAX_TARGETS);

    let mut outcomes = Vec::new();
    let mut stream = stream::iter(work)
        .map(|target| {
            let secret = secret.clone();
            let stop = Arc::clone(&stop);
            let factory = factory.clone();
            async move {
                if stop.load(Ordering::Relaxed) || cancelled(topology_id, scan_id) {
                    return DeviceOutcome::Cancelled;
                }
                probe_one(&provider, &secret, target, timeout, factory).await
            }
        })
        .buffer_unordered(concurrency);

    while let Some(outcome) = stream.next().await {
        if Instant::now() >= deadline || cancelled(topology_id, scan_id) {
            stop.store(true, Ordering::Relaxed);
        }
        outcomes.push(outcome);
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
    drop(stream);

    ACTIVE_TOPOLOGY.store(0, Ordering::Relaxed);

    let was_cancelled = cancelled(topology_id, scan_id);
    let timed_out = started.elapsed() >= Duration::from_millis(MAX_TOTAL_MS);

    let mut views = Vec::new();
    let mut failures = Vec::new();
    let mut responded = 0usize;
    let mut failed = 0usize;
    let queried = outcomes.len();
    for outcome in outcomes {
        match outcome {
            DeviceOutcome::View(view) => {
                responded += 1;
                views.push(*view);
            }
            DeviceOutcome::Failed { ip, reason } => {
                failed += 1;
                failures.push(TopologyDeviceFailure { ip, reason });
            }
            DeviceOutcome::Cancelled => {}
        }
    }

    let snapshot = correlate(&views, &inventory, &captured_at);
    let mut confirmed = 0usize;
    let mut strong = 0usize;
    let mut inferred = 0usize;
    for conn in &snapshot.connections {
        match conn.confidence {
            TopologyConfidence::Confirmed => confirmed += 1,
            TopologyConfidence::Strong => strong += 1,
            TopologyConfidence::Inferred => inferred += 1,
        }
    }

    let unknown_nodes = snapshot.unknown_nodes.len();
    TopologyResult {
        snapshot,
        summary: TopologySummary {
            devices_queried: queried,
            devices_responded: responded,
            devices_failed: failed,
            confirmed,
            strong,
            inferred,
            unknown_nodes,
            duration_ms: started.elapsed().as_millis() as u64,
            cancelled: was_cancelled,
            timed_out,
            failures,
        },
    }
}

pub async fn run_from_request(
    store: &CredentialStore,
    request: TopologyRequest,
) -> Result<TopologyResult, TopologyError> {
    let secret = store.get().ok_or(TopologyError::NotConfigured)?;
    let timeout = Duration::from_millis(
        request
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(100, 5_000),
    );
    let concurrency = request.concurrency.unwrap_or(DEFAULT_CONCURRENCY);
    let inventory = request.targets.clone();
    // SNMP is worth trying on every inventoried address. Hosts that do not
    // speak it time out quickly and are isolated. We do not spray alternative
    // credentials at them.
    let snmp_targets = request.targets;
    Ok(run(
        &secret,
        inventory,
        snmp_targets,
        timeout,
        concurrency,
        request.scan_id,
        None,
    )
    .await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ber::SnmpValue;
    use crate::collect::IF_DESCR;
    use crate::credentials::CredentialInput;
    use crate::snmp::FixtureSession;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn v2c() -> SnmpSecret {
        SnmpSecret::from_input(CredentialInput {
            version: "v2c".into(),
            community: Some("site-read".into()),
            username: None,
            auth_protocol: None,
            auth_password: None,
            priv_protocol: None,
            priv_password: None,
            context: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn one_broken_device_does_not_block_the_others() {
        let secret = v2c();
        let factory: SessionFactory = Arc::new(|ip, _| {
            if ip == Ipv4Addr::new(192, 168, 1, 9) {
                Ok(Box::new(FixtureSession::failing(TopologyError::Timeout)))
            } else {
                let mut values = BTreeMap::new();
                values.insert(
                    "1.3.6.1.2.1.1.5.0".into(),
                    SnmpValue::OctetString(b"core-sw".to_vec()),
                );
                values.insert(
                    format!("{}.1", crate::ber::Oid::from_slice(IF_DESCR)),
                    SnmpValue::OctetString(b"Gi1".to_vec()),
                );
                Ok(Box::new(FixtureSession::new(values)))
            }
        });
        let inventory = vec![
            TopologyTarget {
                ip: "192.168.1.2".into(),
                mac: None,
                device_id: Some(2),
                hostname: Some("core-sw".into()),
                detected_name: None,
            },
            TopologyTarget {
                ip: "192.168.1.9".into(),
                mac: None,
                device_id: Some(9),
                hostname: None,
                detected_name: None,
            },
        ];
        let result = run(
            &secret,
            inventory.clone(),
            inventory,
            Duration::from_millis(200),
            2,
            None,
            Some(factory),
        )
        .await;
        assert_eq!(result.summary.devices_responded, 1);
        assert_eq!(result.summary.devices_failed, 1);
        assert!(!result.summary.failures[0].reason.contains("site-read"));
        assert!(!result.summary.cancelled);
    }

    #[tokio::test]
    async fn missing_credentials_are_a_clean_error() {
        let store = CredentialStore::default();
        let err = run_from_request(
            &store,
            TopologyRequest {
                targets: vec![],
                timeout_ms: None,
                concurrency: None,
                network_name: None,
                scan_id: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TopologyError::NotConfigured));
        assert!(!err.to_string().to_ascii_lowercase().contains("community"));
    }

    #[tokio::test]
    async fn bad_credentials_are_isolated_and_do_not_invite_spraying() {
        let secret = v2c();
        let factory: SessionFactory =
            Arc::new(|_, _| Ok(Box::new(FixtureSession::failing(TopologyError::AuthFailed))));
        let inventory = vec![TopologyTarget {
            ip: "192.168.1.2".into(),
            mac: None,
            device_id: Some(2),
            hostname: Some("core-sw".into()),
            detected_name: None,
        }];
        let result = run(
            &secret,
            inventory.clone(),
            inventory,
            Duration::from_millis(200),
            1,
            None,
            Some(factory),
        )
        .await;
        assert_eq!(result.summary.devices_failed, 1);
        let reason = result.summary.failures[0].reason.to_ascii_lowercase();
        assert!(!reason.contains("site-read"));
        assert!(!reason.contains("try public"));
        assert!(!reason.contains("private"));
    }
}
