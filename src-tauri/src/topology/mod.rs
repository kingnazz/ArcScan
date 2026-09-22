//! Tauri command surface for topology discovery.
//!
//! The engine itself lives in the `arcscan-topology` crate so its tests do not
//! need GTK. This module owns only process state and IPC. Quick Scan does not
//! run this. The frontend combines the returned snapshot with the real
//! Inventory JSON when it builds a schemaVersion 2 ArcAtlas handoff.

use std::sync::Mutex;

use tauri::State;

use arcscan_topology::credentials::{
    CredentialInput, CredentialStatus, CredentialStore, SnmpSecret,
};
use arcscan_topology::engine::{run_from_request_with_capture, TopologyRunCapture};
use arcscan_topology::model::{TopologyRequest, TopologyResult};
use arcscan_topology::replay::ReplayFixture;
use arcscan_topology::serialize::{handoff_preview_to_json, issue42_fixture};

pub use arcscan_topology::engine;

/// Process-wide topology state: session credentials plus an in-memory cache
/// of the last snapshot so the UI can re-read it without re-querying.
pub struct TopologyState {
    credentials: CredentialStore,
    credential_epoch: Mutex<u64>,
    last: Mutex<Option<TopologyResult>>,
    last_capture: Mutex<Option<(TopologyRunCapture, String)>>,
}

impl TopologyState {
    pub fn new() -> Self {
        Self {
            credentials: CredentialStore::default(),
            credential_epoch: Mutex::new(0),
            last: Mutex::new(None),
            last_capture: Mutex::new(None),
        }
    }
}

impl Default for TopologyState {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub fn set_topology_credentials(
    state: State<'_, TopologyState>,
    credentials: CredentialInput,
) -> Result<CredentialStatus, String> {
    let secret = SnmpSecret::from_input(credentials).map_err(String::from)?;
    let mut epoch = state.credential_epoch.lock().expect("credential epoch");
    *epoch = epoch.wrapping_add(1);
    *state.last_capture.lock().expect("topology replay cache") = None;
    let status = state.credentials.set(secret);
    drop(epoch);
    Ok(status)
}

#[tauri::command]
pub fn clear_topology_credentials(state: State<'_, TopologyState>) -> CredentialStatus {
    let mut epoch = state.credential_epoch.lock().expect("credential epoch");
    *epoch = epoch.wrapping_add(1);
    *state.last_capture.lock().expect("topology replay cache") = None;
    let status = state.credentials.clear();
    drop(epoch);
    status
}

#[tauri::command]
pub fn get_topology_credentials(state: State<'_, TopologyState>) -> CredentialStatus {
    state.credentials.status()
}

#[tauri::command]
pub async fn discover_topology(
    state: State<'_, TopologyState>,
    request: TopologyRequest,
) -> Result<TopologyResult, String> {
    let credential_epoch = *state.credential_epoch.lock().expect("credential epoch");
    let mut request = request;
    if request.gateway_ip.is_none() {
        request.gateway_ip = crate::netinfo::default_gateway_ip()
            .await
            .map(|ip| ip.to_string());
    }
    let description = request
        .network_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("ArcScan topology run for {name}"))
        .unwrap_or_else(|| "ArcScan topology run".into());
    let (result, capture) = run_from_request_with_capture(&state.credentials, request)
        .await
        .map_err(String::from)?;
    *state.last.lock().expect("topology cache") = Some(result.clone());
    let current_epoch = state.credential_epoch.lock().expect("credential epoch");
    if *current_epoch == credential_epoch {
        *state.last_capture.lock().expect("topology replay cache") = Some((capture, description));
    }
    drop(current_epoch);
    Ok(result)
}

#[tauri::command]
pub fn last_topology_snapshot(state: State<'_, TopologyState>) -> Option<TopologyResult> {
    state.last.lock().expect("topology cache").clone()
}

#[tauri::command]
pub fn cancel_topology() {
    engine::request_cancel();
}

/// Build a machine-replayable fixture only after the operator explicitly asks
/// for one. The live path keeps parsed evidence in memory and does not pay the
/// serialization cost on every discovery run.
#[tauri::command]
pub fn export_topology_replay_fixture(state: State<'_, TopologyState>) -> Result<String, String> {
    let _epoch = state.credential_epoch.lock().expect("credential epoch");
    let guard = state.last_capture.lock().expect("topology replay cache");
    let (capture, description) = guard
        .as_ref()
        .ok_or_else(|| "Run topology discovery before exporting a replay fixture.".to_string())?;
    let fixture = ReplayFixture::from_capture(capture, description.clone());
    let secret = state.credentials.get().ok_or_else(|| {
        "The credentials used for this topology run were cleared; run topology discovery again before exporting a replay fixture.".to_string()
    })?;
    fixture
        .to_sanitized_json(Some(&secret))
        .map_err(|error| error.to_string())
}

/// Golden serializer fixture for issue #42. No network, no credentials.
#[tauri::command]
pub fn topology_contract_fixture() -> Result<String, String> {
    handoff_preview_to_json(&issue42_fixture()).map_err(|e| e.to_string())
}
