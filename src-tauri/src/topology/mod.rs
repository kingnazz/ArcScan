//! Tauri command surface for topology discovery.
//!
//! The engine itself lives in the `arcscan-topology` crate so its tests do not
//! need GTK. This module owns only process state and IPC. Quick Scan does not
//! run this. The current ArcAtlas inventory envelope is not modified.

use std::sync::Mutex;

use tauri::State;

use arcscan_topology::credentials::{
    CredentialInput, CredentialStatus, CredentialStore, SnmpSecret,
};
use arcscan_topology::engine::run_from_request;
use arcscan_topology::model::{TopologyRequest, TopologyResult};
use arcscan_topology::serialize::{handoff_preview_to_json, issue42_fixture};

pub use arcscan_topology::engine;

/// Process-wide topology state: session credentials plus an in-memory cache
/// of the last snapshot so the UI can re-read it without re-querying.
pub struct TopologyState {
    pub credentials: CredentialStore,
    last: Mutex<Option<TopologyResult>>,
}

impl TopologyState {
    pub fn new() -> Self {
        Self {
            credentials: CredentialStore::default(),
            last: Mutex::new(None),
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
    Ok(state.credentials.set(secret))
}

#[tauri::command]
pub fn clear_topology_credentials(state: State<'_, TopologyState>) -> CredentialStatus {
    state.credentials.clear()
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
    let result = run_from_request(&state.credentials, request)
        .await
        .map_err(String::from)?;
    *state.last.lock().expect("topology cache") = Some(result.clone());
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

/// Golden serializer fixture for issue #42. No network, no credentials.
#[tauri::command]
pub fn topology_contract_fixture() -> Result<String, String> {
    handoff_preview_to_json(&issue42_fixture()).map_err(|e| e.to_string())
}
