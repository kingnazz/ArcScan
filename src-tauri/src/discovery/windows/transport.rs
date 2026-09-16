//! Which WSMan transport a credentialed probe should use, and whether there is
//! one at all.
//!
//! # The problem this solves
//!
//! v1.9.0 decided a host was worth a credentialed probe if it had 445 or 3389
//! open, and then always opened a plain `-Protocol Wsman` session. Those two
//! facts do not follow from each other. A perfectly ordinary Windows
//! workstation has file sharing and Remote Desktop on and WinRM off, so every
//! one of them bought a guaranteed authentication attempt that could only time
//! out — on a site with fifty desktops, fifty of them.
//!
//! So "looks like Windows" and "has a management transport ArcScan can reach"
//! are separated. The first decides whether a host is a *candidate*; the second
//! decides whether anything is actually sent, and which listener it goes to.
//!
//! # Why the check is unauthenticated
//!
//! Selection is a TCP connect and nothing more: no credential, no WSMan
//! handshake, no data. It answers one question — is a listener accepting
//! connections on this port — which is exactly the question that decides
//! whether an authenticated attempt has any chance. A host with neither
//! listener is reported as skipped and is never sent a credential, which is
//! also what keeps this from becoming a retry loop against a domain account.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

/// How long a single listener check may take.
///
/// Short on purpose: this runs against every Windows-looking host, and a
/// listener that is there answers a local TCP connect in milliseconds. A host
/// that does not answer in this long is one ArcScan is not going to reach.
pub const REACHABILITY_TIMEOUT: Duration = Duration::from_millis(700);

/// The WinRM HTTP listener. The default in an Active Directory domain.
pub const WSMAN_HTTP_PORT: u16 = 5985;
/// The WinRM HTTPS listener.
pub const WSMAN_HTTPS_PORT: u16 = 5986;

/// The transport a credentialed probe will use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// WinRM over HTTP, port 5985.
    Http,
    /// WinRM over HTTPS, port 5986.
    Https,
}

impl Transport {
    pub fn port(self) -> u16 {
        match self {
            Transport::Http => WSMAN_HTTP_PORT,
            Transport::Https => WSMAN_HTTPS_PORT,
        }
    }

    pub fn uses_ssl(self) -> bool {
        matches!(self, Transport::Https)
    }

    /// How the transport reads in a status line.
    pub fn label(self) -> &'static str {
        match self {
            Transport::Http => "WinRM over HTTP (5985)",
            Transport::Https => "WinRM over HTTPS (5986)",
        }
    }
}

/// Pick a transport from what is listening.
///
/// # Why HTTP is preferred when both are open
///
/// This looks backwards and is not. WinRM over 5985 is *not* an unencrypted
/// channel: Negotiate/Kerberos encrypts the SOAP payload at the message layer,
/// so the credential and the results are protected either way. It is also the
/// default listener that `Enable-PSRemoting` creates and the one a domain is
/// configured for.
///
/// 5986 adds TLS underneath that, but in practice carries a self-signed
/// certificate that no client trusts — and ArcScan does **not** disable
/// certificate validation to get past it (see [`super::script`]). Preferring
/// 5986 when both exist would therefore turn working hosts into certificate
/// errors for no gain in confidentiality.
///
/// So 5986 is used when it is the only listener, which is the case it exists
/// for: a host where HTTP has been deliberately turned off.
pub fn choose(http_open: bool, https_open: bool) -> Option<Transport> {
    match (http_open, https_open) {
        (true, _) => Some(Transport::Http),
        (false, true) => Some(Transport::Https),
        (false, false) => None,
    }
}

/// True when something is accepting TCP connections on `port`.
///
/// Connect and drop. Nothing is written, so this cannot be mistaken for an
/// authentication attempt and leaves no session behind.
pub async fn listener_open(ip: Ipv4Addr, port: u16) -> bool {
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    matches!(
        tokio::time::timeout(REACHABILITY_TIMEOUT, tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

/// Select the transport for one host, using the sweep's findings where they
/// already answer the question and a TCP connect where they do not.
///
/// `open_ports` is what the port sweep found. The WinRM ports are not in any
/// of ArcScan's default port sets, so in practice this almost always falls
/// through to the live check — but when an operator has scanned 5985 or 5986
/// explicitly, that result is reused rather than re-probed.
pub async fn select(ip: Ipv4Addr, open_ports: &[u16]) -> Option<Transport> {
    let swept_http = open_ports.contains(&WSMAN_HTTP_PORT);
    let swept_https = open_ports.contains(&WSMAN_HTTPS_PORT);
    if swept_http || swept_https {
        return choose(swept_http, swept_https);
    }

    // HTTP first, and short-circuit: when 5985 answers there is nothing the
    // 5986 check could change, so it is not made.
    if listener_open(ip, WSMAN_HTTP_PORT).await {
        return Some(Transport::Http);
    }
    if listener_open(ip, WSMAN_HTTPS_PORT).await {
        return Some(Transport::Https);
    }
    None
}

/// Ports that say a host is probably Windows, as opposed to reachable for
/// management.
///
/// Used only to decide which hosts are worth a transport check. None of these
/// is ever treated as evidence that a credentialed probe will work.
pub const WINDOWS_LOOKING_PORTS: &[u16] = &[
    135,  // RPC endpoint mapper
    139,  // NetBIOS session
    445,  // SMB
    3389, // RDP
    WSMAN_HTTP_PORT,
    WSMAN_HTTPS_PORT,
];

/// True when a host is worth checking for a management transport.
pub fn looks_like_windows(open_ports: &[u16]) -> bool {
    open_ports.iter().any(|p| WINDOWS_LOOKING_PORTS.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_wins_when_both_listeners_are_open() {
        // Not a preference for less security: 5985 is message-encrypted by
        // Negotiate, and 5986 usually carries a certificate nothing trusts.
        assert_eq!(choose(true, true), Some(Transport::Http));
        assert_eq!(choose(true, false), Some(Transport::Http));
    }

    #[test]
    fn https_is_used_when_it_is_the_only_listener() {
        assert_eq!(choose(false, true), Some(Transport::Https));
    }

    #[test]
    fn no_listener_means_no_transport_and_therefore_no_attempt() {
        // The case the whole module exists for: a workstation with SMB and RDP
        // and no WinRM must not be sent a credential.
        assert_eq!(choose(false, false), None);
    }

    #[test]
    fn each_transport_knows_its_port_and_whether_it_is_tls() {
        assert_eq!(Transport::Http.port(), 5985);
        assert_eq!(Transport::Https.port(), 5986);
        assert!(!Transport::Http.uses_ssl());
        assert!(Transport::Https.uses_ssl());
    }

    #[test]
    fn smb_and_rdp_make_a_host_a_candidate_and_nothing_more() {
        assert!(looks_like_windows(&[445]));
        assert!(looks_like_windows(&[3389]));
        assert!(looks_like_windows(&[135, 139]));
        // But they say nothing about a management transport, which is the
        // separation this release introduces.
        assert_eq!(choose(false, false), None);
    }

    #[test]
    fn a_host_with_nothing_windows_looking_is_not_a_candidate() {
        assert!(!looks_like_windows(&[22, 80, 443]));
        assert!(!looks_like_windows(&[]));
    }

    #[tokio::test]
    async fn a_swept_winrm_port_is_reused_rather_than_re_probed() {
        // TEST-NET-1, which nothing answers. The result must come from the
        // sweep rather than from a connect, so this returns immediately.
        let started = std::time::Instant::now();
        let chosen = select(Ipv4Addr::new(192, 0, 2, 1), &[445, WSMAN_HTTP_PORT]).await;
        assert_eq!(chosen, Some(Transport::Http));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn a_swept_https_only_host_selects_tls_without_probing() {
        let started = std::time::Instant::now();
        let chosen = select(Ipv4Addr::new(192, 0, 2, 1), &[WSMAN_HTTPS_PORT]).await;
        assert_eq!(chosen, Some(Transport::Https));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn a_host_with_no_listener_selects_nothing_within_the_budget() {
        // Two bounded connects and no more.
        let started = std::time::Instant::now();
        let chosen = select(Ipv4Addr::new(192, 0, 2, 1), &[445, 3389]).await;
        assert_eq!(chosen, None);
        assert!(started.elapsed() < REACHABILITY_TIMEOUT * 2 + Duration::from_millis(500));
    }

    #[test]
    fn the_reachability_budget_stays_small() {
        // This runs against every Windows-looking host on the network.
        assert!(REACHABILITY_TIMEOUT <= Duration::from_secs(1));
    }
}
