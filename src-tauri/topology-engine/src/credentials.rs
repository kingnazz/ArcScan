//! Session-only SNMP credentials.
//!
//! Technician-entered, never guessed, never sprayed, never written to disk,
//! never returned in a DTO, never included in an error string. The store is
//! process memory. A Portable session already forgets everything at exit;
//! this matches that contract for Installed too, for this PR.

use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::error::{redact_secrets, TopologyError};

/// What the UI is allowed to know about stored credentials: that they exist,
/// which SNMP version they use, and whether a username is present. Never the
/// community, never a password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub configured: bool,
    pub version: Option<String>,
    pub username: Option<String>,
    pub auth_protocol: Option<String>,
    pub priv_protocol: Option<String>,
    pub session_only: bool,
}

impl CredentialStatus {
    pub fn empty() -> Self {
        Self {
            configured: false,
            version: None,
            username: None,
            auth_protocol: None,
            priv_protocol: None,
            session_only: true,
        }
    }
}

/// Wire input from the webview. Dropped after conversion into [`SnmpSecret`].
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInput {
    pub version: String,
    #[serde(default)]
    pub community: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub auth_protocol: Option<String>,
    #[serde(default)]
    pub auth_password: Option<String>,
    #[serde(default)]
    pub priv_protocol: Option<String>,
    #[serde(default)]
    pub priv_password: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
}

impl fmt::Debug for CredentialInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialInput")
            .field("version", &self.version)
            .field("community", &redacted(&self.community))
            .field("username", &self.username.as_deref().map(|_| "[set]"))
            .field("auth_protocol", &self.auth_protocol)
            .field("auth_password", &redacted(&self.auth_password))
            .field("priv_protocol", &self.priv_protocol)
            .field("priv_password", &redacted(&self.priv_password))
            .field("context", &self.context.as_deref().map(|_| "[set]"))
            .finish()
    }
}

fn redacted(value: &Option<String>) -> &'static str {
    if value.as_deref().map(str::is_empty) == Some(false) {
        "[redacted]"
    } else {
        "[absent]"
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnmpVersion {
    V2c,
    V3,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthProtocol {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl AuthProtocol {
    pub fn parse(raw: &str) -> Result<Self, TopologyError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "md5" => Ok(Self::Md5),
            "sha" | "sha1" => Ok(Self::Sha1),
            "sha224" => Ok(Self::Sha224),
            "sha256" => Ok(Self::Sha256),
            "sha384" => Ok(Self::Sha384),
            "sha512" => Ok(Self::Sha512),
            _ => Err(TopologyError::InvalidInput(
                "Choose an SNMPv3 authentication protocol ArcScan supports: MD5, SHA-1, SHA-224, SHA-256, SHA-384 or SHA-512.".into(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrivProtocol {
    Des,
    Aes128,
    Aes192,
    Aes256,
}

impl PrivProtocol {
    pub fn parse(raw: &str) -> Result<Self, TopologyError> {
        match raw.trim().to_ascii_lowercase().replace('-', "").as_str() {
            "des" => Ok(Self::Des),
            "aes" | "aes128" => Ok(Self::Aes128),
            "aes192" => Ok(Self::Aes192),
            "aes256" => Ok(Self::Aes256),
            _ => Err(TopologyError::InvalidInput(
                "Choose an SNMPv3 privacy protocol ArcScan supports: DES, AES-128, AES-192 or AES-256.".into(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Des => "des",
            Self::Aes128 => "aes128",
            Self::Aes192 => "aes192",
            Self::Aes256 => "aes256",
        }
    }
}

/// In-memory secret. `Debug` is redacted so a panic log cannot leak it.
#[derive(Clone)]
pub enum SnmpSecret {
    V2c {
        community: Vec<u8>,
    },
    V3 {
        username: String,
        auth_protocol: Option<AuthProtocol>,
        auth_password: Option<Vec<u8>>,
        priv_protocol: Option<PrivProtocol>,
        priv_password: Option<Vec<u8>>,
        context: Option<String>,
    },
}

impl fmt::Debug for SnmpSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V2c { .. } => f
                .debug_struct("SnmpSecret::V2c")
                .field("community", &"[redacted]")
                .finish(),
            Self::V3 {
                username,
                auth_protocol,
                priv_protocol,
                ..
            } => f
                .debug_struct("SnmpSecret::V3")
                .field("username", &"[redacted]")
                .field("auth_protocol", auth_protocol)
                .field("priv_protocol", priv_protocol)
                .field("username_len", &username.len())
                .finish(),
        }
    }
}

impl SnmpSecret {
    pub fn version(&self) -> SnmpVersion {
        match self {
            Self::V2c { .. } => SnmpVersion::V2c,
            Self::V3 { .. } => SnmpVersion::V3,
        }
    }

    pub fn status(&self) -> CredentialStatus {
        match self {
            Self::V2c { .. } => CredentialStatus {
                configured: true,
                version: Some("v2c".into()),
                username: None,
                auth_protocol: None,
                priv_protocol: None,
                session_only: true,
            },
            Self::V3 {
                username,
                auth_protocol,
                priv_protocol,
                ..
            } => CredentialStatus {
                configured: true,
                version: Some("v3".into()),
                // The username is an identifier, not a password, but the
                // contract says credentials never appear in DTOs. Report only
                // that a user is configured.
                username: if username.is_empty() {
                    None
                } else {
                    Some("[configured]".into())
                },
                auth_protocol: auth_protocol.map(AuthProtocol::as_str).map(str::to_string),
                priv_protocol: priv_protocol.map(PrivProtocol::as_str).map(str::to_string),
                session_only: true,
            },
        }
    }

    pub fn from_input(input: CredentialInput) -> Result<Self, TopologyError> {
        match input.version.trim().to_ascii_lowercase().as_str() {
            "v2c" | "2c" | "snmpv2c" => {
                let community = input.community.unwrap_or_default();
                let community = community.trim();
                if community.is_empty() {
                    return Err(TopologyError::InvalidInput(
                        "Enter an SNMP community string. ArcScan never tries public, private or any other default.".into(),
                    ));
                }
                if community.len() > 256 {
                    return Err(TopologyError::InvalidInput(
                        "That community string is unreasonably long.".into(),
                    ));
                }
                Ok(Self::V2c {
                    community: community.as_bytes().to_vec(),
                })
            }
            "v3" | "3" | "snmpv3" => {
                let username = input.username.unwrap_or_default();
                let username = username.trim();
                if username.is_empty() {
                    return Err(TopologyError::InvalidInput(
                        "Enter an SNMPv3 username.".into(),
                    ));
                }
                if username.len() > 64 {
                    return Err(TopologyError::InvalidInput(
                        "That SNMPv3 username is unreasonably long.".into(),
                    ));
                }
                let auth_protocol = match input
                    .auth_protocol
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(raw) => Some(AuthProtocol::parse(raw)?),
                    None => None,
                };
                let auth_password = nonempty_secret(input.auth_password.as_deref());
                let priv_protocol = match input
                    .priv_protocol
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(raw) => Some(PrivProtocol::parse(raw)?),
                    None => None,
                };
                let priv_password = nonempty_secret(input.priv_password.as_deref());

                if priv_protocol.is_some() && auth_protocol.is_none() {
                    return Err(TopologyError::InvalidInput(
                        "SNMPv3 privacy requires authentication. Choose an authentication protocol too.".into(),
                    ));
                }
                if auth_protocol.is_some() && auth_password.is_none() {
                    return Err(TopologyError::InvalidInput(
                        "Enter the SNMPv3 authentication password.".into(),
                    ));
                }
                if priv_protocol.is_some() && priv_password.is_none() {
                    return Err(TopologyError::InvalidInput(
                        "Enter the SNMPv3 privacy password.".into(),
                    ));
                }
                if auth_protocol.is_none() {
                    return Err(TopologyError::InvalidInput(
                        "ArcScan does not send SNMPv3 with noAuthNoPriv. Choose authentication, and privacy when the device requires it.".into(),
                    ));
                }

                Ok(Self::V3 {
                    username: username.to_string(),
                    auth_protocol,
                    auth_password,
                    priv_protocol,
                    priv_password,
                    context: input
                        .context
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                })
            }
            _ => Err(TopologyError::InvalidInput(
                "Choose SNMP v2c or SNMP v3.".into(),
            )),
        }
    }
}

fn nonempty_secret(raw: Option<&str>) -> Option<Vec<u8>> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.as_bytes().to_vec())
}

/// Process-wide session store. One technician's credentials, for this run.
pub struct CredentialStore {
    inner: Mutex<Option<SnmpSecret>>,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

impl CredentialStore {
    pub fn set(&self, secret: SnmpSecret) -> CredentialStatus {
        let status = secret.status();
        *self.inner.lock().expect("credential store mutex") = Some(secret);
        status
    }

    pub fn clear(&self) -> CredentialStatus {
        *self.inner.lock().expect("credential store mutex") = None;
        CredentialStatus::empty()
    }

    pub fn status(&self) -> CredentialStatus {
        self.inner
            .lock()
            .expect("credential store mutex")
            .as_ref()
            .map(SnmpSecret::status)
            .unwrap_or_else(CredentialStatus::empty)
    }

    pub fn get(&self) -> Option<SnmpSecret> {
        self.inner.lock().expect("credential store mutex").clone()
    }
}

/// True when `haystack` contains a secret we must never echo.
pub fn leaks_secret(haystack: &str, secret: &SnmpSecret) -> bool {
    match secret {
        SnmpSecret::V2c { community } => contains_bytes(haystack, community),
        SnmpSecret::V3 {
            username,
            auth_password,
            priv_password,
            context,
            ..
        } => {
            (!username.is_empty() && haystack.contains(username))
                || auth_password
                    .as_ref()
                    .is_some_and(|p| contains_bytes(haystack, p))
                || priv_password
                    .as_ref()
                    .is_some_and(|p| contains_bytes(haystack, p))
                || context
                    .as_ref()
                    .is_some_and(|c| !c.is_empty() && haystack.contains(c))
        }
    }
}

fn contains_bytes(haystack: &str, secret: &[u8]) -> bool {
    if secret.is_empty() {
        return false;
    }
    if let Ok(s) = std::str::from_utf8(secret) {
        if !s.is_empty() && haystack.contains(s) {
            return true;
        }
    }
    // Also catch hex dumps of the secret bytes.
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    haystack.to_ascii_lowercase().contains(&hex)
}

pub fn sanitize_text(text: String, secret: Option<&SnmpSecret>) -> String {
    let mut out = redact_secrets(&text);
    if let Some(secret) = secret {
        out = match secret {
            SnmpSecret::V2c { community } => wipe(&out, community),
            SnmpSecret::V3 {
                username,
                auth_password,
                priv_password,
                context,
                ..
            } => {
                let mut s = wipe(&out, username.as_bytes());
                if let Some(p) = auth_password {
                    s = wipe(&s, p);
                }
                if let Some(p) = priv_password {
                    s = wipe(&s, p);
                }
                if let Some(c) = context {
                    s = wipe(&s, c.as_bytes());
                }
                s
            }
        };
    }
    out
}

fn wipe(text: &str, secret: &[u8]) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    if let Ok(s) = std::str::from_utf8(secret) {
        if !s.is_empty() {
            out = out.replace(s, "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2c(community: &str) -> CredentialInput {
        CredentialInput {
            version: "v2c".into(),
            community: Some(community.into()),
            username: None,
            auth_protocol: None,
            auth_password: None,
            priv_protocol: None,
            priv_password: None,
            context: None,
        }
    }

    #[test]
    fn refuses_empty_community() {
        let err = SnmpSecret::from_input(v2c("")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("community"));
        assert!(!msg.to_ascii_lowercase().contains("try public"));
    }

    #[test]
    fn never_tries_a_default_community_for_the_operator() {
        // The only community that gets stored is the one they typed. An empty
        // field is an error, not an implicit `public`.
        assert!(SnmpSecret::from_input(v2c("   ")).is_err());
        let secret = SnmpSecret::from_input(v2c("site-read")).unwrap();
        match secret {
            SnmpSecret::V2c { community } => assert_eq!(community, b"site-read"),
            _ => panic!("expected v2c"),
        }
    }

    #[test]
    fn status_does_not_echo_the_community() {
        let secret = SnmpSecret::from_input(v2c("super-secret-community")).unwrap();
        let status = secret.status();
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("super-secret"));
        assert!(status.configured);
        assert_eq!(status.version.as_deref(), Some("v2c"));
        assert!(status.session_only);
    }

    #[test]
    fn v3_requires_auth_and_does_not_echo_passwords() {
        let input = CredentialInput {
            version: "v3".into(),
            community: None,
            username: Some("monitor".into()),
            auth_protocol: Some("sha256".into()),
            auth_password: Some("auth-secret-value".into()),
            priv_protocol: Some("aes128".into()),
            priv_password: Some("priv-secret-value".into()),
            context: None,
        };
        let secret = SnmpSecret::from_input(input).unwrap();
        let json = serde_json::to_string(&secret.status()).unwrap();
        assert!(!json.contains("auth-secret"));
        assert!(!json.contains("priv-secret"));
        assert!(!json.contains("monitor"));
        assert!(json.contains("sha256"));
        assert!(json.contains("aes128"));
    }

    #[test]
    fn v3_noauth_is_refused() {
        let input = CredentialInput {
            version: "v3".into(),
            community: None,
            username: Some("monitor".into()),
            auth_protocol: None,
            auth_password: None,
            priv_protocol: None,
            priv_password: None,
            context: None,
        };
        assert!(SnmpSecret::from_input(input).is_err());
    }

    #[test]
    fn v3_auth_no_priv_is_accepted() {
        let input = CredentialInput {
            version: "v3".into(),
            community: None,
            username: Some("monitor".into()),
            auth_protocol: Some("sha256".into()),
            auth_password: Some("auth-secret-value".into()),
            priv_protocol: None,
            priv_password: None,
            context: Some("vlan-10".into()),
        };
        let secret = SnmpSecret::from_input(input).unwrap();
        let status = secret.status();
        assert_eq!(status.auth_protocol.as_deref(), Some("sha256"));
        assert!(status.priv_protocol.is_none());
        assert!(!format!("{secret:?}").contains("auth-secret-value"));
    }

    #[test]
    fn debug_format_redacts_community() {
        let secret = SnmpSecret::from_input(v2c("leaky-community")).unwrap();
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("leaky-community"));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn sanitize_strips_community_from_errors() {
        let secret = SnmpSecret::from_input(v2c("leaky-community")).unwrap();
        let cleaned = sanitize_text(
            "authentication failed for community leaky-community on 10.0.0.1".into(),
            Some(&secret),
        );
        assert!(!cleaned.contains("leaky-community"));
        assert!(leaks_secret("community leaky-community rejected", &secret));
        assert!(!leaks_secret(&cleaned, &secret));
    }

    #[test]
    fn store_is_session_memory() {
        let store = CredentialStore::default();
        assert!(!store.status().configured);
        store.set(SnmpSecret::from_input(v2c("site-read")).unwrap());
        assert!(store.status().configured);
        store.clear();
        assert!(!store.status().configured);
        assert!(store.get().is_none());
    }
}
