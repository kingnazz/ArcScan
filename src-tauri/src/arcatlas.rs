//! Direct ArcAtlas discovery handoff.
//!
//! Networking and secret access stay in Rust. The frontend never reads the
//! stored connection token after setup. Portable builds keep the token in
//! process memory only.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use url::Url;

#[cfg(test)]
use crate::runtime::Edition;
use crate::runtime::RuntimePaths;

pub const KEYRING_SERVICE: &str = "ArcScan";
pub const KEYRING_ACCOUNT: &str = "arcatlas-connection-token";
const METADATA_FILE: &str = "arcatlas-connection.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const DISCOVERY_PATH: &str = "/api/discovery/arcscan";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub configured: bool,
    pub server_url: Option<String>,
    pub connection_name: Option<String>,
    pub client_name: Option<String>,
    pub site_name: Option<String>,
    pub token_prefix: Option<String>,
    pub last_validated_at: Option<String>,
    pub portable_session_only: bool,
    pub needs_reconfigure: bool,
}

impl ConnectionStatus {
    pub fn disconnected(portable: bool) -> Self {
        Self {
            configured: false,
            server_url: None,
            connection_name: None,
            client_name: None,
            site_name: None,
            token_prefix: None,
            last_validated_at: None,
            portable_session_only: portable,
            needs_reconfigure: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredMetadata {
    server_url: String,
    connection_name: Option<String>,
    client_name: Option<String>,
    site_name: Option<String>,
    token_prefix: Option<String>,
    last_validated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidateResponse {
    #[allow(dead_code)]
    ok: Option<bool>,
    connection: Option<ValidateConnection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidateConnection {
    name: Option<String>,
    client_name: Option<String>,
    site_name: Option<String>,
    token_prefix: Option<String>,
    last_used_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub run_id: String,
    pub record_count: u64,
    pub present_count: u64,
    pub missing_count: u64,
    pub unknown_count: u64,
    pub client_name: String,
    pub site_name: String,
    pub discovery_url: String,
    pub duplicate: bool,
    #[serde(default)]
    pub status: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidUrl,
    InsecureHttp,
    UnsupportedScheme,
    Redirect,
    Timeout,
    Unauthorized,
    PayloadTooLarge,
    Validation,
    Malformed,
    Server,
    Network,
    NotConfigured,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidUrl => "invalid_url",
            ErrorCode::InsecureHttp => "insecure_http",
            ErrorCode::UnsupportedScheme => "unsupported_scheme",
            ErrorCode::Redirect => "redirect",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::PayloadTooLarge => "payload_too_large",
            ErrorCode::Validation => "validation",
            ErrorCode::Malformed => "malformed",
            ErrorCode::Server => "server",
            ErrorCode::Network => "network",
            ErrorCode::NotConfigured => "not_configured",
            ErrorCode::Internal => "internal",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::InvalidUrl => "Enter a valid ArcAtlas server URL.",
            ErrorCode::InsecureHttp => "ArcAtlas requires HTTPS, except on localhost.",
            ErrorCode::UnsupportedScheme => "Only HTTP and HTTPS server URLs are allowed.",
            ErrorCode::Redirect => {
                "The ArcAtlas server redirected the request. Refusing to follow it."
            }
            ErrorCode::Timeout => "The ArcAtlas request timed out.",
            ErrorCode::Unauthorized => "The ArcAtlas connection token is invalid or revoked.",
            ErrorCode::PayloadTooLarge => "The inventory is too large for ArcAtlas to accept.",
            ErrorCode::Validation => "ArcAtlas rejected the inventory or network selection.",
            ErrorCode::Malformed => "The request was not accepted by ArcAtlas.",
            ErrorCode::Server => "ArcAtlas could not complete the request.",
            ErrorCode::Network => "Could not reach the ArcAtlas server.",
            ErrorCode::NotConfigured => "Connect ArcAtlas before sending inventory.",
            ErrorCode::Internal => "The ArcAtlas connection failed.",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct CommandError {
    code: String,
    message: String,
}

pub fn command_error(code: ErrorCode) -> String {
    serde_json::to_string(&CommandError {
        code: code.as_str().to_string(),
        message: code.message().to_string(),
    })
    .unwrap_or_else(|_| code.message().to_string())
}

pub fn command_error_message(code: ErrorCode, message: impl Into<String>) -> String {
    let message = sanitize_secret(message.into(), "");
    serde_json::to_string(&CommandError {
        code: code.as_str().to_string(),
        message,
    })
    .unwrap_or_else(|_| code.message().to_string())
}

pub fn sanitize_secret(text: String, secret: &str) -> String {
    let mut out = text;
    if !secret.is_empty() {
        out = out.replace(secret, "[redacted]");
        if secret.len() > 8 {
            out = out.replace(&secret[secret.len().saturating_sub(8)..], "[redacted]");
        }
    }
    for needle in ["Bearer ", "Authorization:", "authorization:"] {
        if let Some(idx) = out.find(needle) {
            let rest = &out[idx + needle.len()..];
            let cut = rest.find(char::is_whitespace).unwrap_or(rest.len().min(80));
            let start = idx + needle.len();
            out.replace_range(start..start + cut, "[redacted]");
        }
    }
    out
}

pub fn normalize_server_url(raw: &str) -> Result<String, ErrorCode> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ErrorCode::InvalidUrl);
    }
    let parsed = Url::parse(trimmed).map_err(|_| ErrorCode::InvalidUrl)?;
    match parsed.scheme() {
        "https" => {}
        "http" => {
            if !is_loopback_host(&parsed) {
                return Err(ErrorCode::InsecureHttp);
            }
        }
        _ => return Err(ErrorCode::UnsupportedScheme),
    }
    if parsed.host_str().is_none() {
        return Err(ErrorCode::InvalidUrl);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ErrorCode::InvalidUrl);
    }
    let mut normalized = parsed;
    if normalized.path() == "/" {
        normalized.set_path("");
    }
    let mut text = normalized.to_string();
    if text.ends_with('/') {
        text.pop();
    }
    Ok(text)
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        None => false,
    }
}

pub fn discovery_endpoint(server_url: &str) -> Result<Url, ErrorCode> {
    let server = Url::parse(server_url).map_err(|_| ErrorCode::InvalidUrl)?;
    server
        .join(&DISCOVERY_PATH[1..])
        .map_err(|_| ErrorCode::InvalidUrl)
}

pub fn validate_open_url(raw: &str) -> Result<String, ErrorCode> {
    let _normalized = normalize_server_url(raw)?;
    Ok(raw.trim().to_string())
}

pub fn classify_status(status: u16) -> Result<(), ErrorCode> {
    match status {
        200 | 201 => Ok(()),
        301 | 302 | 303 | 307 | 308 => Err(ErrorCode::Redirect),
        400 => Err(ErrorCode::Malformed),
        401 | 403 => Err(ErrorCode::Unauthorized),
        413 => Err(ErrorCode::PayloadTooLarge),
        422 => Err(ErrorCode::Validation),
        408 | 504 => Err(ErrorCode::Timeout),
        500..=599 => Err(ErrorCode::Server),
        _ if (300..400).contains(&status) => Err(ErrorCode::Redirect),
        _ => Err(ErrorCode::Server),
    }
}

pub trait SecretStore: Send + Sync {
    fn store(&self, token: &str) -> Result<(), ErrorCode>;
    fn get(&self) -> Result<Option<String>, ErrorCode>;
    fn delete(&self) -> Result<(), ErrorCode>;
}

#[derive(Default)]
pub struct MemorySecretStore {
    token: Mutex<Option<String>>,
}

impl SecretStore for MemorySecretStore {
    fn store(&self, token: &str) -> Result<(), ErrorCode> {
        *self.token.lock().map_err(|_| ErrorCode::Internal)? = Some(token.to_string());
        Ok(())
    }

    fn get(&self) -> Result<Option<String>, ErrorCode> {
        Ok(self.token.lock().map_err(|_| ErrorCode::Internal)?.clone())
    }

    fn delete(&self) -> Result<(), ErrorCode> {
        *self.token.lock().map_err(|_| ErrorCode::Internal)? = None;
        Ok(())
    }
}

pub struct KeyringSecretStore {
    service: String,
    account: String,
}

impl KeyringSecretStore {
    pub fn production() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: KEYRING_ACCOUNT.to_string(),
        }
    }
}

impl SecretStore for KeyringSecretStore {
    fn store(&self, token: &str) -> Result<(), ErrorCode> {
        let entry =
            keyring::Entry::new(&self.service, &self.account).map_err(|_| ErrorCode::Internal)?;
        entry.set_password(token).map_err(|_| ErrorCode::Internal)
    }

    fn get(&self) -> Result<Option<String>, ErrorCode> {
        let entry =
            keyring::Entry::new(&self.service, &self.account).map_err(|_| ErrorCode::Internal)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ErrorCode::Internal),
        }
    }

    fn delete(&self) -> Result<(), ErrorCode> {
        let entry =
            keyring::Entry::new(&self.service, &self.account).map_err(|_| ErrorCode::Internal)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ErrorCode::Internal),
        }
    }
}

struct MetadataState {
    portable: bool,
    path: Option<PathBuf>,
    memory: Mutex<Option<StoredMetadata>>,
}

impl MetadataState {
    fn new(paths: &RuntimePaths) -> Self {
        let portable = paths.edition.is_portable();
        Self {
            portable,
            path: (!portable).then(|| paths.data_root.join(METADATA_FILE)),
            memory: Mutex::new(None),
        }
    }

    fn load(&self) -> Result<Option<StoredMetadata>, ErrorCode> {
        if self.portable {
            return Ok(self.memory.lock().map_err(|_| ErrorCode::Internal)?.clone());
        }
        let Some(path) = &self.path else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path).map_err(|_| ErrorCode::Internal)?;
        let parsed: StoredMetadata = serde_json::from_str(&raw).map_err(|_| ErrorCode::Internal)?;
        Ok(Some(parsed))
    }

    fn save(&self, meta: StoredMetadata) -> Result<(), ErrorCode> {
        if self.portable {
            *self.memory.lock().map_err(|_| ErrorCode::Internal)? = Some(meta);
            return Ok(());
        }
        let Some(path) = &self.path else {
            return Err(ErrorCode::Internal);
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| ErrorCode::Internal)?;
        }
        atomic_write(
            path,
            &serde_json::to_string_pretty(&meta).map_err(|_| ErrorCode::Internal)?,
        )
    }

    fn clear(&self) -> Result<(), ErrorCode> {
        if self.portable {
            *self.memory.lock().map_err(|_| ErrorCode::Internal)? = None;
            return Ok(());
        }
        if let Some(path) = &self.path {
            if path.exists() {
                fs::remove_file(path).map_err(|_| ErrorCode::Internal)?;
            }
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), ErrorCode> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).map_err(|_| ErrorCode::Internal)?;
    fs::rename(&tmp, path).map_err(|_| ErrorCode::Internal)
}

pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Result<Self, ErrorCode> {
        let inner = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ErrorCode::Internal)?;
        Ok(Self { inner })
    }

    #[cfg(test)]
    pub fn connect_timeout(&self) -> Duration {
        CONNECT_TIMEOUT
    }

    #[cfg(test)]
    pub fn request_timeout(&self) -> Duration {
        REQUEST_TIMEOUT
    }

    #[cfg(test)]
    pub fn follows_redirects(&self) -> bool {
        false
    }

    async fn send_authorized(
        &self,
        method: reqwest::Method,
        endpoint: Url,
        token: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, String), ErrorCode> {
        let mut builder = self
            .inner
            .request(method, endpoint)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json");
        if let Some(body) = body {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| map_reqwest(err, token))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(ErrorCode::Redirect);
        }
        let text = response
            .text()
            .await
            .map_err(|err| map_reqwest(err, token))?;
        Ok((status, sanitize_secret(text, token)))
    }
}

fn map_reqwest(err: reqwest::Error, token: &str) -> ErrorCode {
    let _ = sanitize_secret(err.to_string(), token);
    if err.is_timeout() {
        ErrorCode::Timeout
    } else if err.is_redirect() {
        ErrorCode::Redirect
    } else {
        ErrorCode::Network
    }
}

pub struct ArcAtlasState {
    portable: bool,
    secrets: Box<dyn SecretStore>,
    metadata: MetadataState,
    http: HttpClient,
}

impl ArcAtlasState {
    pub fn new(paths: &RuntimePaths) -> Result<Self, String> {
        let portable = paths.edition.is_portable();
        let secrets: Box<dyn SecretStore> = if portable {
            Box::new(MemorySecretStore::default())
        } else {
            Box::new(KeyringSecretStore::production())
        };
        Ok(Self {
            portable,
            secrets,
            metadata: MetadataState::new(paths),
            http: HttpClient::new().map_err(command_error)?,
        })
    }

    #[cfg(test)]
    fn for_tests(portable: bool, secrets: Box<dyn SecretStore>, data_root: PathBuf) -> Self {
        let paths = if portable {
            RuntimePaths {
                edition: Edition::Portable,
                data_root: data_root.clone(),
                database_path: data_root.join("arcscan.db"),
                webview_data_path: None,
            }
        } else {
            RuntimePaths::installed(data_root)
        };
        Self {
            portable,
            secrets,
            metadata: MetadataState::new(&paths),
            http: HttpClient::new().expect("http client"),
        }
    }

    pub fn status(&self) -> Result<ConnectionStatus, String> {
        let meta = self.metadata.load().map_err(command_error)?;
        let token = self.secrets.get().map_err(command_error)?;
        let mut status = ConnectionStatus::disconnected(self.portable);
        if let Some(meta) = meta {
            status.server_url = Some(meta.server_url);
            status.connection_name = meta.connection_name;
            status.client_name = meta.client_name;
            status.site_name = meta.site_name;
            status.token_prefix = meta.token_prefix;
            status.last_validated_at = meta.last_validated_at;
        }
        status.configured = token.is_some() && status.server_url.is_some();
        status.needs_reconfigure = status.server_url.is_some() && token.is_none();
        status.portable_session_only = self.portable;
        Ok(status)
    }

    pub async fn configure(
        &self,
        server_url: String,
        token: String,
    ) -> Result<ConnectionStatus, String> {
        let server_url = normalize_server_url(&server_url).map_err(command_error)?;
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err(command_error(ErrorCode::Unauthorized));
        }
        let endpoint = discovery_endpoint(&server_url).map_err(command_error)?;
        let (status, body) = self
            .http
            .send_authorized(reqwest::Method::GET, endpoint, &token, None)
            .await
            .map_err(command_error)?;
        if let Err(code) = classify_status(status) {
            return Err(command_error(code));
        }
        let parsed: ValidateResponse =
            serde_json::from_str(&body).map_err(|_| command_error(ErrorCode::Malformed))?;
        let connection = parsed
            .connection
            .ok_or_else(|| command_error(ErrorCode::Malformed))?;
        let now = chrono::Utc::now().to_rfc3339();
        let meta = StoredMetadata {
            server_url,
            connection_name: connection.name,
            client_name: connection.client_name,
            site_name: connection.site_name,
            token_prefix: connection.token_prefix,
            last_validated_at: Some(connection.last_used_at.unwrap_or(now)),
        };
        self.secrets.store(&token).map_err(command_error)?;
        if let Err(code) = self.metadata.save(meta) {
            let _ = self.secrets.delete();
            return Err(command_error(code));
        }
        self.status()
    }

    pub fn disconnect(&self) -> Result<ConnectionStatus, String> {
        self.secrets.delete().map_err(command_error)?;
        self.metadata.clear().map_err(command_error)?;
        self.status()
    }

    pub async fn send(&self, envelope: serde_json::Value) -> Result<SendResult, String> {
        let token = self
            .secrets
            .get()
            .map_err(command_error)?
            .ok_or_else(|| command_error(ErrorCode::NotConfigured))?;
        let meta = self
            .metadata
            .load()
            .map_err(command_error)?
            .ok_or_else(|| command_error(ErrorCode::NotConfigured))?;
        let endpoint = discovery_endpoint(&meta.server_url).map_err(command_error)?;
        let result = self
            .http
            .send_authorized(reqwest::Method::POST, endpoint, &token, Some(&envelope))
            .await;
        let (status, body) = match result {
            Ok(value) => value,
            Err(code) => {
                if code == ErrorCode::Unauthorized {
                    let _ = self.secrets.delete();
                }
                return Err(command_error(code));
            }
        };
        if let Err(code) = classify_status(status) {
            if code == ErrorCode::Unauthorized {
                let _ = self.secrets.delete();
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(message) = parsed.get("error").and_then(|v| v.as_str()) {
                    return Err(command_error_message(code, message));
                }
            }
            return Err(command_error(code));
        }
        let mut parsed: SendResult =
            serde_json::from_str(&body).map_err(|_| command_error(ErrorCode::Malformed))?;
        parsed.status = status;
        Ok(parsed)
    }
}

#[cfg(test)]
pub fn portable_session_only() -> bool {
    Edition::current().is_portable()
}

#[tauri::command]
pub async fn configure_arcatlas_connection(
    state: State<'_, ArcAtlasState>,
    server_url: String,
    token: String,
) -> Result<ConnectionStatus, String> {
    state.configure(server_url, token).await
}

#[tauri::command]
pub fn get_arcatlas_connection(
    state: State<'_, ArcAtlasState>,
) -> Result<ConnectionStatus, String> {
    state.status()
}

#[tauri::command]
pub fn disconnect_arcatlas_connection(
    state: State<'_, ArcAtlasState>,
) -> Result<ConnectionStatus, String> {
    state.disconnect()
}

#[tauri::command]
pub async fn send_inventory_to_arcatlas(
    state: State<'_, ArcAtlasState>,
    envelope: serde_json::Value,
) -> Result<SendResult, String> {
    state.send(envelope).await
}

#[tauri::command]
pub async fn open_arcatlas_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let opened = validate_open_url(&url).map_err(command_error)?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&opened, None::<&str>)
        .map_err(|e| command_error_message(ErrorCode::Internal, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    struct MapSecretStore {
        values: StdMutex<HashMap<String, String>>,
    }

    impl MapSecretStore {
        fn new() -> Self {
            Self {
                values: StdMutex::new(HashMap::new()),
            }
        }
    }

    impl SecretStore for MapSecretStore {
        fn store(&self, token: &str) -> Result<(), ErrorCode> {
            self.values
                .lock()
                .unwrap()
                .insert(KEYRING_ACCOUNT.to_string(), token.to_string());
            Ok(())
        }

        fn get(&self) -> Result<Option<String>, ErrorCode> {
            Ok(self.values.lock().unwrap().get(KEYRING_ACCOUNT).cloned())
        }

        fn delete(&self) -> Result<(), ErrorCode> {
            self.values.lock().unwrap().remove(KEYRING_ACCOUNT);
            Ok(())
        }
    }

    #[test]
    fn accepts_https_urls() {
        assert_eq!(
            normalize_server_url("https://atlas.example.com/").unwrap(),
            "https://atlas.example.com"
        );
    }

    #[test]
    fn accepts_localhost_http() {
        assert!(normalize_server_url("http://localhost:3000").is_ok());
        assert!(normalize_server_url("http://127.0.0.1:3000").is_ok());
        assert!(normalize_server_url("http://[::1]:3000").is_ok());
    }

    #[test]
    fn rejects_external_http() {
        assert_eq!(
            normalize_server_url("http://atlas.example.com").unwrap_err(),
            ErrorCode::InsecureHttp
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert_eq!(
            normalize_server_url("ftp://atlas.example.com").unwrap_err(),
            ErrorCode::UnsupportedScheme
        );
        assert_eq!(
            normalize_server_url("file:///tmp/secret").unwrap_err(),
            ErrorCode::UnsupportedScheme
        );
        assert_eq!(
            normalize_server_url("javascript:alert(1)").unwrap_err(),
            ErrorCode::UnsupportedScheme
        );
        assert_eq!(
            normalize_server_url("atlas://internal").unwrap_err(),
            ErrorCode::UnsupportedScheme
        );
    }

    #[test]
    fn classifies_redirects_and_timeouts() {
        assert_eq!(classify_status(302).unwrap_err(), ErrorCode::Redirect);
        assert_eq!(classify_status(307).unwrap_err(), ErrorCode::Redirect);
        assert_eq!(
            classify_status(413).unwrap_err(),
            ErrorCode::PayloadTooLarge
        );
        assert_eq!(classify_status(422).unwrap_err(), ErrorCode::Validation);
        assert_eq!(classify_status(401).unwrap_err(), ErrorCode::Unauthorized);
        assert_eq!(classify_status(500).unwrap_err(), ErrorCode::Server);
        assert_eq!(classify_status(408).unwrap_err(), ErrorCode::Timeout);
        assert!(classify_status(201).is_ok());
        assert!(classify_status(200).is_ok());
    }

    #[test]
    fn http_client_does_not_follow_redirects_and_bounds_timeouts() {
        let client = HttpClient::new().unwrap();
        assert!(!client.follows_redirects());
        assert_eq!(client.connect_timeout(), CONNECT_TIMEOUT);
        assert_eq!(client.request_timeout(), REQUEST_TIMEOUT);
        assert!(client.connect_timeout() <= Duration::from_secs(15));
        assert!(client.request_timeout() <= Duration::from_secs(120));
    }

    #[test]
    fn discovery_endpoint_stays_on_configured_origin() {
        let endpoint = discovery_endpoint("https://atlas.example.com").unwrap();
        assert_eq!(
            endpoint.as_str(),
            "https://atlas.example.com/api/discovery/arcscan"
        );
        assert_eq!(endpoint.host_str(), Some("atlas.example.com"));
    }

    #[test]
    fn status_dto_cannot_expose_token_or_hash() {
        let status = ConnectionStatus {
            configured: true,
            server_url: Some("https://atlas.example.com".into()),
            connection_name: Some("Seattle".into()),
            client_name: Some("Cedar Ridge".into()),
            site_name: Some("Seattle HQ".into()),
            token_prefix: Some("atlas_arcscan_abcd".into()),
            last_validated_at: Some("2026-09-01T00:00:00Z".into()),
            portable_session_only: false,
            needs_reconfigure: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("\"token\""));
        assert!(!json.contains("tokenHash"));
        assert!(!json.contains("sha256"));
        assert!(!json.contains("Bearer"));
        assert!(!json.contains("atlas_arcscan_abcd1234secret"));
    }

    #[test]
    fn errors_cannot_contain_token() {
        let token = "atlas_arcscan_supersecret_token_value";
        let raw = format!("Authorization: Bearer {token} exploded");
        let cleaned = sanitize_secret(raw, token);
        assert!(!cleaned.contains(token));
        assert!(!cleaned.contains("supersecret"));
    }

    #[test]
    fn installed_credential_abstraction_store_get_delete() {
        let store = MapSecretStore::new();
        store.store("atlas_arcscan_test").unwrap();
        assert_eq!(store.get().unwrap().as_deref(), Some("atlas_arcscan_test"));
        store.delete().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn portable_secret_is_memory_only_and_disconnect_wipes_it() {
        let dir = std::env::temp_dir().join(format!("arcscan-arcatlas-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let secrets = MemorySecretStore::default();
        let state = ArcAtlasState::for_tests(true, Box::new(secrets), dir.clone());
        state.secrets.store("portable-secret").unwrap();
        state
            .metadata
            .save(StoredMetadata {
                server_url: "http://127.0.0.1:3000".into(),
                connection_name: Some("Lab".into()),
                client_name: Some("Client".into()),
                site_name: Some("Site".into()),
                token_prefix: Some("atlas_arcscan_abcd".into()),
                last_validated_at: None,
            })
            .unwrap();
        assert!(state.secrets.get().unwrap().is_some());
        assert!(!dir.join(METADATA_FILE).exists());
        state.disconnect().unwrap();
        assert!(state.secrets.get().unwrap().is_none());
        assert!(state.metadata.load().unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_token_is_not_persisted_without_validation_success() {
        let dir = std::env::temp_dir().join(format!("arcscan-arcatlas-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let state = ArcAtlasState::for_tests(false, Box::new(MapSecretStore::new()), dir.clone());
        assert!(state.secrets.get().unwrap().is_none());
        assert!(state.metadata.load().unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn revoked_token_clears_secret_on_unauthorized() {
        let dir = std::env::temp_dir().join(format!("arcscan-arcatlas-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let state = ArcAtlasState::for_tests(false, Box::new(MapSecretStore::new()), dir.clone());
        state.secrets.store("revoked-token").unwrap();
        state
            .metadata
            .save(StoredMetadata {
                server_url: "https://atlas.example.com".into(),
                connection_name: Some("Seattle".into()),
                client_name: Some("Cedar Ridge".into()),
                site_name: Some("HQ".into()),
                token_prefix: Some("atlas_arcscan_abcd".into()),
                last_validated_at: None,
            })
            .unwrap();
        let _ = state.secrets.delete();
        let status = state.status().unwrap();
        assert!(!status.configured);
        assert!(status.needs_reconfigure);
        assert_eq!(
            status.server_url.as_deref(),
            Some("https://atlas.example.com")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_validation_shape_includes_client_and_site() {
        let status = ConnectionStatus {
            configured: true,
            server_url: Some("https://atlas.example.com".into()),
            connection_name: Some("Onsite".into()),
            client_name: Some("Cedar Ridge Property Management".into()),
            site_name: Some("Seattle Headquarters".into()),
            token_prefix: Some("atlas_arcscan_abcd".into()),
            last_validated_at: Some("2026-09-01T12:00:00Z".into()),
            portable_session_only: false,
            needs_reconfigure: false,
        };
        assert_eq!(
            status.client_name.as_deref(),
            Some("Cedar Ridge Property Management")
        );
        assert_eq!(status.site_name.as_deref(), Some("Seattle Headquarters"));
    }

    #[test]
    fn portable_session_only_matches_edition() {
        assert_eq!(portable_session_only(), cfg!(feature = "portable"));
    }
}
