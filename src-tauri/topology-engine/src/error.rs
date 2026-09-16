//! Topology errors. Every message that crosses the IPC boundary has already
//! been stripped of anything that looks like a credential.

use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    InvalidInput(String),
    NotConfigured,
    Timeout,
    AuthFailed,
    Unreachable,
    Protocol(String),
    Cancelled,
    Internal(String),
}

impl TopologyError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotConfigured => "not_configured",
            Self::Timeout => "timeout",
            Self::AuthFailed => "auth_failed",
            Self::Unreachable => "unreachable",
            Self::Protocol(_) => "protocol",
            Self::Cancelled => "cancelled",
            Self::Internal(_) => "internal",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidInput(msg) => redact_secrets(msg),
            Self::NotConfigured => {
                "Enter SNMP credentials before discovering topology.".into()
            }
            Self::Timeout => {
                "The device did not answer SNMP in time. That can mean it is not an SNMP agent, or that the credentials are wrong — ArcScan cannot tell those apart, and will not guess another community.".into()
            }
            Self::AuthFailed => {
                "SNMP authentication failed for this device.".into()
            }
            Self::Unreachable => {
                "ArcScan could not reach this device over SNMP.".into()
            }
            Self::Protocol(msg) => redact_secrets(msg),
            Self::Cancelled => "Topology discovery was stopped.".into(),
            Self::Internal(msg) => redact_secrets(msg),
        }
    }
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for TopologyError {}

/// IPC-facing error. `code` is stable; `message` is already sanitized.
#[derive(Debug, Clone, Serialize)]
pub struct TopologyErrorDto {
    pub code: String,
    pub message: String,
}

impl From<TopologyError> for TopologyErrorDto {
    fn from(err: TopologyError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.message(),
        }
    }
}

impl From<TopologyError> for String {
    fn from(err: TopologyError) -> Self {
        err.message()
    }
}

/// Strip substrings that look like SNMP community strings or passwords from
/// an error we did not ourselves compose. Deliberately blunt: a false positive
/// that hides a word is better than leaking a secret.
pub fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for needle in [
        "community ",
        "community=",
        "authPass",
        "privPass",
        "auth_password",
        "priv_password",
        "privacy password",
        "authentication password",
    ] {
        if let Some(idx) = out.to_ascii_lowercase().find(&needle.to_ascii_lowercase()) {
            let rest = &out[idx + needle.len()..];
            let cut = rest
                .find(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            let start = idx + needle.len();
            out.replace_range(start..start + cut, "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_message_does_not_invite_spraying() {
        let msg = TopologyError::Timeout.message().to_ascii_lowercase();
        assert!(!msg.contains("try public"));
        assert!(!msg.contains("private"));
        assert!(msg.contains("will not guess"));
    }

    #[test]
    fn redacts_community_from_foreign_errors() {
        let cleaned = redact_secrets("snmp: unknown community site-read on agent");
        assert!(!cleaned.contains("site-read"));
        assert!(cleaned.contains("[redacted]"));
    }
}
