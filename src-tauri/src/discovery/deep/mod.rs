//! Deep scanning: richer identity, still unauthenticated, still read-only.
//!
//! # The three levels
//!
//! * **Quick** collects what a fast sweep already produces — address, MAC,
//!   host name, vendor, open ports — plus the multicast discovery ArcScan has
//!   done since v1.8.2. Nothing here runs, and Quick's timing is unchanged.
//! * **Deep** adds this module: for ports the sweep *already found open*, it
//!   asks the service what it is. An HTTP front page, a TLS certificate
//!   subject, an SMB negotiation, a service greeting.
//! * **Credentialed deep** adds [`super::windows`] on top, which is the only
//!   level that can name an exact Windows edition.
//!
//! # The rules this module works under
//!
//! * **Only open ports.** Deep probes never widen the port scan. If the sweep
//!   did not find 443 open, nothing here connects to 443.
//! * **Bounded everywhere.** Every probe has its own connect and read deadline
//!   and a fixed read ceiling, and the whole pass has a budget on top. A device
//!   that accepts a connection and goes silent costs one timeout.
//! * **Read-only.** The probes send a `GET /`, a TLS ClientHello, an SMB
//!   NEGOTIATE, or nothing at all. No credential is offered, no form is
//!   submitted, no share is enumerated, no second guess is made.
//! * **Nothing invented.** Every probe records what it actually saw. A device
//!   that says nothing recognisable still gets its banner written down; it does
//!   not get a type.

pub mod banner;
pub mod der;
pub mod fingerprint;
pub mod httpfp;
pub mod smbfp;
pub mod tlsfp;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::discovery::model::Evidence;

/// How long a single deep probe may take, connection included.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(2_500);
/// How long the connection alone may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(900);
/// Most bytes read from any one probe.
pub const MAX_READ_BYTES: usize = 64 * 1024;
/// Most probes run against a single host, whatever it has open.
///
/// A host with two hundred open ports is not worth two hundred connections;
/// the first few answer the question or nothing will.
pub const MAX_PROBES_PER_HOST: usize = 8;

/// Ports asked for an HTTP front page, when the sweep found them open.
pub const HTTP_PORTS: &[u16] = &[80, 8080, 8000, 8008, 81, 280, 591];
/// Ports offered a TLS ClientHello, when the sweep found them open.
pub const TLS_PORTS: &[u16] = &[443, 8443, 9443, 5001, 4443, 10000];
/// The SMB port.
pub const SMB_PORT: u16 = 445;

/// Which deep probes the operator has switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeepOptions {
    /// The master switch for the whole level.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "on")]
    pub http: bool,
    #[serde(default = "on")]
    pub tls: bool,
    #[serde(default = "on")]
    pub smb: bool,
    #[serde(default = "on")]
    pub banners: bool,
}

fn on() -> bool {
    true
}

impl Default for DeepOptions {
    fn default() -> Self {
        // Off by default, which is what keeps Quick Scan quick. A request from
        // a build that predates deep scanning deserializes to exactly this.
        DeepOptions {
            enabled: false,
            http: true,
            tls: true,
            smb: true,
            banners: true,
        }
    }
}

impl DeepOptions {
    /// Every probe on. The Deep Scan profile's setting.
    pub fn all() -> Self {
        DeepOptions {
            enabled: true,
            ..Default::default()
        }
    }
}

/// What one deep pass produced for one host.
#[derive(Debug, Clone, Default)]
pub struct DeepOutcome {
    pub evidence: Vec<Evidence>,
    /// One line per probe attempted, for the history view. Says what was tried
    /// and what came back, so a technician can tell "nothing answered" apart
    /// from "nothing was asked".
    pub notes: Vec<String>,
}

impl DeepOutcome {
    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }
}

/// Run every enabled deep probe against one host.
///
/// `open_ports` comes from the sweep that already happened. This never adds a
/// port to it.
pub async fn probe_host(ip: Ipv4Addr, open_ports: &[u16], options: &DeepOptions) -> DeepOutcome {
    let mut outcome = DeepOutcome::default();
    if !options.enabled {
        return outcome;
    }
    let mut budget = MAX_PROBES_PER_HOST;

    if options.http {
        for port in HTTP_PORTS.iter().copied().filter(|p| open_ports.contains(p)) {
            if budget == 0 {
                break;
            }
            budget -= 1;
            match http_probe(ip, port).await {
                Ok(fingerprint) => {
                    outcome.evidence.extend(httpfp::evidence(&fingerprint, port));
                    outcome
                        .notes
                        .push(format!("HTTP on {port}: {} ", fingerprint.status));
                }
                Err(reason) => outcome.notes.push(format!("HTTP on {port}: {reason}")),
            }
            // One answering web server is enough. A device serving the same
            // interface on 80 and 8080 has not said two things.
            if !outcome.evidence.is_empty() {
                break;
            }
        }
    }

    if options.tls {
        for port in TLS_PORTS.iter().copied().filter(|p| open_ports.contains(p)) {
            if budget == 0 {
                break;
            }
            budget -= 1;
            match tls_probe(ip, port).await {
                Ok(fingerprint) => {
                    outcome.evidence.extend(tlsfp::evidence(&fingerprint, port));
                    outcome.notes.push(format!(
                        "TLS on {port}: {}",
                        fingerprint.common_name.as_deref().unwrap_or("no subject")
                    ));
                    break;
                }
                Err(reason) => outcome.notes.push(format!("TLS on {port}: {reason}")),
            }
        }
    }

    if options.smb && open_ports.contains(&SMB_PORT) && budget > 0 {
        budget -= 1;
        match smb_probe(ip).await {
            Ok(fingerprint) => {
                outcome.evidence.extend(smbfp::evidence(&fingerprint));
                outcome.notes.push(format!(
                    "SMB on {SMB_PORT}: {}",
                    fingerprint.dialect.label().unwrap_or("unknown dialect")
                ));
            }
            Err(reason) => outcome.notes.push(format!("SMB on {SMB_PORT}: {reason}")),
        }
    }

    if options.banners {
        for port in banner::GREETING_PORTS
            .iter()
            .copied()
            .filter(|p| open_ports.contains(p))
        {
            if budget == 0 {
                break;
            }
            budget -= 1;
            match banner_probe(ip, port).await {
                Ok(text) => {
                    outcome.evidence.extend(banner::evidence(&text, port));
                    outcome.notes.push(format!("Banner on {port}: {text}"));
                }
                Err(reason) => outcome.notes.push(format!("Banner on {port}: {reason}")),
            }
        }
    }

    outcome
}

/// Connect, optionally send, read until the deadline or the ceiling.
///
/// The one piece of socket code in this module. Everything that decides what
/// the bytes *mean* lives in the sibling modules, where it can be tested
/// against fixtures.
async fn exchange(ip: Ipv4Addr, port: u16, request: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    let work = async {
        let mut stream = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await
        {
            Err(_) => return Err("no answer".to_string()),
            Ok(Err(_)) => return Err("refused".to_string()),
            Ok(Ok(stream)) => stream,
        };
        if let Some(request) = request {
            stream
                .write_all(request)
                .await
                .map_err(|_| "the connection closed while asking".to_string())?;
            let _ = stream.flush().await;
        }
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.len() >= MAX_READ_BYTES {
                        buffer.truncate(MAX_READ_BYTES);
                        break;
                    }
                    // A greeting-only probe has what it came for after the
                    // first read; waiting for the peer to close would cost a
                    // full timeout on every SSH server on the network.
                    if request.is_none() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if buffer.is_empty() {
            return Err("answered with nothing".to_string());
        }
        Ok(buffer)
    };
    match tokio::time::timeout(PROBE_TIMEOUT, work).await {
        Err(_) => Err("timed out".to_string()),
        Ok(result) => result,
    }
}

async fn http_probe(ip: Ipv4Addr, port: u16) -> Result<httpfp::HttpFingerprint, String> {
    let request = httpfp::request_bytes(&ip.to_string());
    let raw = exchange(ip, port, Some(&request)).await?;
    httpfp::parse_response(&raw).ok_or_else(|| "the reply was not HTTP".to_string())
}

async fn tls_probe(ip: Ipv4Addr, port: u16) -> Result<tlsfp::TlsFingerprint, String> {
    let hello = tlsfp::client_hello();
    let raw = exchange(ip, port, Some(&hello)).await?;
    let certificate = tlsfp::first_certificate(&raw)
        .ok_or_else(|| "no certificate was presented".to_string())?;
    tlsfp::parse_certificate(&certificate)
        .ok_or_else(|| "the certificate had no readable subject".to_string())
}

async fn smb_probe(ip: Ipv4Addr) -> Result<smbfp::SmbFingerprint, String> {
    let request = smbfp::negotiate_request();
    let raw = exchange(ip, SMB_PORT, Some(&request)).await?;
    smbfp::parse_negotiate_response(&raw)
        .ok_or_else(|| "the reply was not an SMB2 negotiation".to_string())
}

async fn banner_probe(ip: Ipv4Addr, port: u16) -> Result<String, String> {
    let raw = exchange(ip, port, None).await?;
    banner::parse_banner(&raw).ok_or_else(|| "the greeting was empty".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_scanning_is_off_unless_it_is_asked_for() {
        // The guarantee that keeps Quick Scan quick, and that makes a request
        // from an older build deserialize to Quick behaviour.
        assert!(!DeepOptions::default().enabled);
        let from_old_build: DeepOptions = serde_json::from_str("{}").unwrap();
        assert!(!from_old_build.enabled);
    }

    #[test]
    fn the_deep_profile_turns_every_probe_on() {
        let options = DeepOptions::all();
        assert!(options.enabled);
        assert!(options.http && options.tls && options.smb && options.banners);
    }

    #[tokio::test]
    async fn a_disabled_pass_opens_no_socket_and_produces_nothing() {
        let outcome = probe_host(
            Ipv4Addr::new(192, 0, 2, 1),
            &[80, 443, 445],
            &DeepOptions::default(),
        )
        .await;
        assert!(outcome.is_empty());
        assert!(outcome.notes.is_empty());
    }

    #[tokio::test]
    async fn a_host_with_no_open_ports_is_never_connected_to() {
        // The rule that deep probes never widen the port scan: an empty open
        // port list means nothing is attempted, so this returns immediately
        // rather than spending a timeout per candidate port.
        let started = std::time::Instant::now();
        let outcome = probe_host(Ipv4Addr::new(192, 0, 2, 1), &[], &DeepOptions::all()).await;
        assert!(outcome.is_empty());
        assert!(outcome.notes.is_empty());
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn switching_one_probe_off_leaves_the_others_alone() {
        let options = DeepOptions {
            smb: false,
            ..DeepOptions::all()
        };
        assert!(options.enabled);
        assert!(!options.smb);
        assert!(options.http);
    }

    #[test]
    fn the_probe_ceilings_are_bounded() {
        // A host with two hundred open ports must not become two hundred
        // connections.
        assert!(MAX_PROBES_PER_HOST <= 16);
        assert!(PROBE_TIMEOUT <= Duration::from_secs(5));
        assert!(MAX_READ_BYTES <= 128 * 1024);
    }
}
