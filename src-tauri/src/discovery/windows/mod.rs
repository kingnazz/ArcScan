//! Optional credentialed Windows discovery.
//!
//! # What this is for
//!
//! Every other part of ArcScan infers. This part asks. Given credentials the
//! operator typed, it opens an authenticated CIM session to one Windows machine
//! and reads what that machine says about itself: the exact edition, version,
//! build and architecture, the manufacturer, model, service tag and system
//! UUID, the domain it is joined to, and — the fact the whole v1.9 effort turns
//! on — `Win32_OperatingSystem.ProductType`.
//!
//! `ProductType` is what separates a workstation from a server, reported by the
//! operating system itself. A Windows 11 laptop with file sharing and Remote
//! Desktop switched on looks exactly like a server from the outside, and it is
//! not one. No amount of port evidence settles that; one authenticated field
//! does.
//!
//! # What it is not
//!
//! It is not a way in. There is one credential, the operator supplied it, and
//! there is no code path that tries a second one — no default list, no
//! fallback account, no retry with a different password. Credential spraying is
//! not disabled here; it is absent.
//!
//! # Platform
//!
//! The collection path is Windows-only, and deliberately so: it runs the
//! machine's own PowerShell CIM stack rather than reimplementing DCOM or WinRM.
//! On every other platform [`probe`] returns [`WindowsError::Unsupported`],
//! which reads as a clear failure. It never returns an empty success, because
//! "we asked and learned nothing" and "we could not ask" are different answers
//! and only one of them is worth showing an operator.

// The collection path — the script generator, the process launcher and the
// parsers that read its output — is only *called* from the `cfg(windows)`
// `probe` below. It is nonetheless compiled and tested on every platform, on
// purpose: everything that can actually be wrong about a credentialed scan is
// in the parsing, and pinning that down on a Linux CI runner is worth far more
// than compiling it only where it runs.
//
// So on a non-Windows build these items are genuinely unreferenced, and saying
// so here is more honest than adding a fake caller. On Windows the allow is
// absent, so real dead code there still fails the build.
#![cfg_attr(not(windows), allow(dead_code))]

pub mod creds;
pub mod facts;
pub mod parse;
pub mod release;
pub mod script;
pub mod transport;

pub use creds::{CredentialStatus, CredentialStore, WindowsCredential};
pub use facts::{WindowsFacts, WindowsProductType};
pub use transport::Transport;

use std::net::Ipv4Addr;

/// Longest a single machine's credentialed probe may take.
pub const PROBE_TIMEOUT_SECS: u64 = 30;

/// The one credential store, for this run of the process.
///
/// Process-wide rather than threaded through the scan, because its lifetime is
/// exactly the process's: it is created on first use, it is never written
/// anywhere, and it ceases to exist when ArcScan exits. Giving it a narrower
/// scope would mean copying the credential to reach the scanner, and the fewer
/// copies of a password exist the better.
///
/// There is no loader. Nothing populates this except an operator typing into
/// the credentialed-scan dialog, so there is no path by which a stored,
/// inherited or guessed credential could appear in it.
pub fn credential_store() -> &'static CredentialStore {
    static STORE: std::sync::OnceLock<CredentialStore> = std::sync::OnceLock::new();
    STORE.get_or_init(CredentialStore::new)
}

/// Why a credentialed probe did not produce facts.
///
/// Every variant renders as a sentence an operator can act on. None of them
/// carries a credential, and none of them is silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsError {
    /// This build of ArcScan cannot run a credentialed Windows scan.
    Unsupported(String),
    /// No credential has been supplied this session.
    NoCredential,
    /// The address was refused before anything was sent.
    InvalidTarget(String),
    /// The machine rejected the credential.
    AccessDenied(String),
    /// The machine did not answer in time.
    Timeout,
    /// PowerShell could not be started at all.
    Launch(String),
    /// The machine answered, and the answer could not be used.
    Unreadable(String),
    /// The machine answered with an error of its own.
    Remote(String),
    /// No WinRM listener answered, so nothing was sent.
    ///
    /// Not a failure of the credential and not an error the operator has to
    /// act on host by host: it is the ordinary state of a Windows workstation
    /// with remote management switched off.
    NoManagementTransport,
    /// The scan was stopped while this probe was in flight.
    Cancelled,
    /// The HTTPS listener presented a certificate that did not validate.
    ///
    /// Its own variant because the remedy is specific and because ArcScan
    /// deliberately will not work around it by disabling validation.
    CertificateRejected(String),
}

impl WindowsError {
    /// The message shown in the drawer and the history view.
    pub fn reason(&self) -> String {
        match self {
            WindowsError::Unsupported(detail) => detail.clone(),
            WindowsError::NoCredential => {
                "No Windows credential is set for this session, so the credentialed scan was \
                 skipped."
                    .into()
            }
            WindowsError::InvalidTarget(detail) => detail.clone(),
            WindowsError::AccessDenied(detail) => {
                format!("The machine refused the credential: {detail}")
            }
            WindowsError::Timeout => {
                "The machine did not answer the management query in time.".into()
            }
            WindowsError::Launch(detail) => {
                format!("ArcScan could not start the Windows management query: {detail}")
            }
            WindowsError::Unreadable(detail) => {
                format!("The machine's reply could not be read: {detail}")
            }
            WindowsError::Remote(detail) => detail.clone(),
            WindowsError::NoManagementTransport => {
                "No WinRM listener answered on 5985 or 5986, so no credential was sent. \
                 Remote management is switched off on this machine."
                    .into()
            }
            WindowsError::Cancelled => "The scan was stopped before this machine answered.".into(),
            WindowsError::CertificateRejected(detail) => format!(
                "The machine's WinRM certificate did not validate, so ArcScan stopped rather \
                 than send a credential to an unverified listener: {detail}"
            ),
        }
    }

    /// True when trying the same machine again might work.
    ///
    /// A refused credential is not retryable, which is what keeps a failed
    /// scan from turning into repeated authentication attempts against a
    /// domain account that locks out.
    pub fn retryable(&self) -> bool {
        matches!(self, WindowsError::Timeout | WindowsError::Launch(_))
    }

    /// True when nothing was sent to the machine at all.
    ///
    /// These are not failures to report per host: a workstation with remote
    /// management off is the normal case, and a stopped scan is the operator's
    /// own doing. Counting them as failures would bury a genuinely refused
    /// credential in noise.
    pub fn is_skip(&self) -> bool {
        matches!(
            self,
            WindowsError::NoManagementTransport
                | WindowsError::Cancelled
                | WindowsError::NoCredential
        )
    }
}

/// Whether this build can run a credentialed Windows scan.
///
/// `Ok(())` on Windows, and a plain-words refusal everywhere else. Called by
/// the credential store so the interface can say up front that the feature is
/// unavailable rather than offering a button that always fails.
pub fn platform_support() -> Result<(), String> {
    #[cfg(windows)]
    {
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(format!(
            "Credentialed Windows discovery needs the Windows build of ArcScan. This is the {} \
             build, which has no Windows management stack to query through.",
            std::env::consts::OS
        ))
    }
}

/// Refuse a target that is not a bare IPv4 address.
///
/// The address is interpolated into a PowerShell script, so the only safe
/// answer is to accept nothing that is not already an `Ipv4Addr`. Parsing and
/// re-rendering, rather than pattern-matching the string, is what guarantees
/// that: whatever comes out is four numbers and three dots.
pub fn validate_target(target: &str) -> Result<String, WindowsError> {
    match target.trim().parse::<Ipv4Addr>() {
        Ok(addr) => Ok(addr.to_string()),
        Err(_) => Err(WindowsError::InvalidTarget(format!(
            "{} is not an IPv4 address ArcScan will query.",
            target.trim()
        ))),
    }
}

/// Refuse a host name that is not a plain DNS name.
///
/// Like [`validate_target`], this exists because the value is interpolated
/// into a PowerShell script. Reverse DNS is attacker-influenced on a network
/// ArcScan does not control, so the name is held to the letters, digits,
/// hyphens and dots a host name is made of, with the label and total length
/// limits DNS itself imposes. Anything else falls back to the address.
pub fn validate_hostname(hostname: &str) -> Option<String> {
    let trimmed = hostname.trim().trim_end_matches('.');
    if trimmed.is_empty() || trimmed.len() > 253 {
        return None;
    }
    // A name that parses as an address is an address, and belongs in the other
    // validator.
    if trimmed.parse::<Ipv4Addr>().is_ok() {
        return None;
    }
    for label in trimmed.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return None;
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return None;
        }
    }
    Some(trimmed.to_string())
}

/// Where a credentialed probe should connect, and by what name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    /// The address the sweep found. Always present, and the fallback.
    pub ip: Ipv4Addr,
    /// A validated host name, when reverse DNS produced a usable one.
    pub hostname: Option<String>,
}

impl ProbeTarget {
    /// Build a target, validating the host name and discarding it if it is not
    /// a plain DNS name.
    pub fn new(ip: Ipv4Addr, hostname: Option<&str>) -> Self {
        ProbeTarget {
            ip,
            hostname: hostname.and_then(validate_hostname),
        }
    }

    /// The name to hand PowerShell.
    ///
    /// The host name wins when there is one, because Kerberos authenticates
    /// against a service principal name built from the host name: connecting
    /// by address cannot find an SPN, so the attempt falls back to NTLM, which
    /// a hardened domain often refuses outright. Connecting by name is the
    /// difference between working and "access denied" on a correctly
    /// configured network.
    ///
    /// When there is no usable name the address is used and the attempt may
    /// still succeed over NTLM; that fallback is deliberate and reported as
    /// whatever the machine says about it.
    pub fn connect_name(&self) -> String {
        match &self.hostname {
            Some(hostname) => hostname.clone(),
            None => self.ip.to_string(),
        }
    }

    /// True when the probe will connect by name rather than by address.
    pub fn is_kerberos_friendly(&self) -> bool {
        self.hostname.is_some()
    }
}

/// Classify a failure reported by the remote machine or by PowerShell.
///
/// Matched on the message because the CIM stack reports an HRESULT in prose;
/// the point is to tell "wrong password" apart from "host is down", so the
/// interface can say which without an operator reading an 0x8007 code.
pub fn classify_failure(message: &str) -> WindowsError {
    let lower = message.to_lowercase();
    let denied = [
        "access is denied",
        "access denied",
        "logon failure",
        "authentication",
        "unauthorized",
        "0x80070005",
        "the user name or password is incorrect",
        "account is locked",
        "credential",
    ];
    if denied.iter().any(|needle| lower.contains(needle)) {
        return WindowsError::AccessDenied(message.trim().to_string());
    }
    // Checked before the generic cases: a certificate failure is reported as
    // one so the operator knows the remedy is a trusted certificate rather
    // than a different password.
    let certificate = [
        "certificate",
        "ssl",
        "cn name",
        "x509",
        "trust relationship",
    ];
    if certificate.iter().any(|needle| lower.contains(needle)) {
        return WindowsError::CertificateRejected(message.trim().to_string());
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return WindowsError::Timeout;
    }
    WindowsError::Remote(message.trim().to_string())
}

/// How often the in-flight wait re-checks for a stop request.
///
/// A probe can sit for the whole of [`PROBE_TIMEOUT_SECS`], and Stop has to
/// take effect inside that window rather than after it.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Ask one Windows machine about itself.
///
/// The credential is borrowed under the store's lock for exactly as long as it
/// takes to write it to the child process's stdin, and is never copied into an
/// argument, an environment variable, a file or a log line.
///
/// # Child process lifecycle
///
/// The child is spawned with `kill_on_drop`, and on every path that does not
/// end in the process exiting on its own — the deadline, a stop request — it is
/// killed explicitly and reaped before this returns. Dropping a wait future is
/// not on its own a guarantee that a WinRM session on the other machine stops,
/// so neither is relied on alone.
///
/// `is_cancelled` is polled while the probe is in flight, so Stop takes effect
/// inside the timeout window instead of leaving sessions running for the
/// remainder of it.
#[cfg(windows)]
pub async fn probe<F>(
    target: &ProbeTarget,
    transport: Transport,
    store: &CredentialStore,
    is_cancelled: F,
) -> Result<WindowsFacts, WindowsError>
where
    F: Fn() -> bool,
{
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Checked on Windows too, not only off it. It is unconditionally `Ok`
    // today, and honouring it here means a future condition — no PowerShell,
    // an edition without WinRM — refuses the probe rather than being
    // discovered as a launch failure per host.
    if let Err(reason) = platform_support() {
        return Err(WindowsError::Unsupported(reason));
    }
    if is_cancelled() {
        return Err(WindowsError::Cancelled);
    }

    // Re-validated here even though `ProbeTarget::new` already did it: this is
    // the last point before the value is interpolated into a script, and the
    // check is cheap. The address is validated by parsing and re-rendering,
    // the host name against the DNS character set.
    let connect_name = target.connect_name();
    let connect_name = if target.is_kerberos_friendly() {
        validate_hostname(&connect_name).ok_or_else(|| {
            WindowsError::InvalidTarget(format!(
                "{connect_name} is not a host name ArcScan will query."
            ))
        })?
    } else {
        validate_target(&connect_name)?
    };

    // The account and the password are taken together, under one lock, so the
    // pair cannot change between building the command and writing the secret.
    let Some((account, password)) =
        store.with(|credential| (credential.account(), credential.password.expose().to_vec()))
    else {
        return Err(WindowsError::NoCredential);
    };

    let encoded = script::encode_command(&script::collection_script(
        &connect_name,
        &account,
        transport,
    ));

    let mut child = crate::scanner::quiet_command("powershell.exe")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-EncodedCommand")
        .arg(&encoded)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The backstop: however this function leaves, the process does not
        // outlive the `Child`.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| WindowsError::Launch(e.to_string()))?;

    // stdin is the only place the password goes. The buffer is zeroed
    // immediately afterwards rather than waiting to be dropped.
    let mut password = password;
    let write = match child.stdin.take() {
        Some(mut stdin) => {
            let result = async {
                stdin.write_all(&password).await?;
                stdin.write_all(b"\r\n").await?;
                stdin.flush().await?;
                stdin.shutdown().await
            }
            .await;
            zeroize(&mut password);
            result.map_err(|e| WindowsError::Launch(e.to_string()))
        }
        None => {
            zeroize(&mut password);
            Err(WindowsError::Launch(
                "the management query would not accept input".into(),
            ))
        }
    };
    if let Err(error) = write {
        terminate(&mut child).await;
        return Err(error);
    }

    // Both pipes are drained concurrently. Reading one to completion before
    // the other would deadlock a child that fills the pipe it is not being
    // read from, which a machine with many network adapters can do.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let drain = async move {
        let read_stdout = async move {
            let mut buffer = Vec::new();
            if let Some(pipe) = stdout_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut buffer).await;
            }
            buffer
        };
        let read_stderr = async move {
            let mut buffer = Vec::new();
            if let Some(pipe) = stderr_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut buffer).await;
            }
            buffer
        };
        tokio::join!(read_stdout, read_stderr)
    };

    let deadline = tokio::time::sleep(std::time::Duration::from_secs(PROBE_TIMEOUT_SECS));
    tokio::pin!(deadline);
    tokio::pin!(drain);

    let collected = loop {
        tokio::select! {
            pair = &mut drain => break Some(pair),
            _ = &mut deadline => break None,
            _ = tokio::time::sleep(CANCEL_POLL) => {
                if is_cancelled() {
                    terminate(&mut child).await;
                    return Err(WindowsError::Cancelled);
                }
            }
        }
    };

    let Some((stdout, stderr)) = collected else {
        // The deadline. Kill and reap before returning, so no WinRM session is
        // left running for the remainder of its own timeout.
        terminate(&mut child).await;
        return Err(WindowsError::Timeout);
    };

    // The pipes are at EOF, so the process has finished or closed them. Reap it
    // rather than leaving a zombie, bounded so a wedged child cannot hang the
    // scan here either.
    match tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await {
        Ok(_) => {}
        Err(_) => terminate(&mut child).await,
    }

    let text = String::from_utf8_lossy(&stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        let detail = String::from_utf8_lossy(&stderr);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            WindowsError::Unreadable("the machine returned nothing".into())
        } else {
            classify_failure(detail)
        });
    }

    match parse::parse_report(trimmed) {
        Ok(facts) => Ok(facts),
        // A parse failure that carried a message from the machine is the
        // machine's answer, not a parser bug, and reads better as such.
        Err(detail) => Err(classify_failure(&detail)),
    }
}

/// Kill a child and wait for it, so the call returns with the process gone.
///
/// `start_kill` then `wait`: the wait is what reaps it and what makes "this
/// function returned" mean "that PowerShell is not still talking to a domain
/// controller". Errors are ignored because every one of them means the process
/// is already gone.
#[cfg(windows)]
async fn terminate(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
}

/// Overwrite a buffer through a volatile write, so it is not optimised away.
#[cfg(windows)]
fn zeroize(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

/// The non-Windows answer: a clear refusal.
///
/// Not an empty `WindowsFacts`, and not a silent skip. A technician running the
/// macOS or Linux build who asks for a credentialed scan is told why it did not
/// happen.
#[cfg(not(windows))]
pub async fn probe<F>(
    target: &ProbeTarget,
    _transport: Transport,
    _store: &CredentialStore,
    is_cancelled: F,
) -> Result<WindowsFacts, WindowsError>
where
    F: Fn() -> bool,
{
    // Validated first so the refusal for a malformed target is the same on
    // every platform, which keeps the tests honest.
    let connect_name = target.connect_name();
    if target.is_kerberos_friendly() {
        validate_hostname(&connect_name).ok_or_else(|| {
            WindowsError::InvalidTarget(format!(
                "{connect_name} is not a host name ArcScan will query."
            ))
        })?;
    } else {
        validate_target(&connect_name)?;
    }
    if is_cancelled() {
        return Err(WindowsError::Cancelled);
    }
    Err(WindowsError::Unsupported(
        platform_support().unwrap_err_or_default(),
    ))
}

/// Small helper so the `cfg(not(windows))` arm above reads in one line.
#[cfg(not(windows))]
trait UnwrapErrOrDefault {
    fn unwrap_err_or_default(self) -> String;
}

#[cfg(not(windows))]
impl UnwrapErrOrDefault for Result<(), String> {
    fn unwrap_err_or_default(self) -> String {
        self.err().unwrap_or_else(|| {
            "Credentialed Windows discovery is unavailable in this build.".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bare_ipv4_address_is_accepted_as_a_target() {
        assert_eq!(validate_target("10.0.0.5").unwrap(), "10.0.0.5");
        assert_eq!(validate_target("  192.168.1.20 ").unwrap(), "192.168.1.20");
    }

    #[test]
    fn anything_that_could_reach_a_shell_is_refused() {
        for hostile in [
            "10.0.0.5; Start-Process calc",
            "10.0.0.5'",
            "$(whoami)",
            "server.corp.example",
            "10.0.0.5 -Credential x",
            "",
            "10.0.0.5/24",
        ] {
            assert!(
                matches!(
                    validate_target(hostile),
                    Err(WindowsError::InvalidTarget(_))
                ),
                "{hostile} must be refused"
            );
        }
    }

    #[test]
    fn a_refused_credential_is_not_retried() {
        let denied = classify_failure("Access is denied. (0x80070005)");
        assert!(matches!(denied, WindowsError::AccessDenied(_)));
        // The rule that keeps a scan from locking out a domain account.
        assert!(!denied.retryable());
    }

    #[test]
    fn a_logon_failure_reads_as_access_denied() {
        assert!(matches!(
            classify_failure("Logon failure: unknown user name or bad password."),
            WindowsError::AccessDenied(_)
        ));
    }

    #[test]
    fn a_timeout_is_retryable_and_a_remote_error_is_not() {
        assert!(matches!(
            classify_failure("The operation timed out"),
            WindowsError::Timeout
        ));
        assert!(WindowsError::Timeout.retryable());
        assert!(!WindowsError::Remote("WinRM cannot process the request".into()).retryable());
    }

    #[test]
    fn every_failure_says_something_an_operator_can_read() {
        for error in [
            WindowsError::NoCredential,
            WindowsError::Timeout,
            WindowsError::AccessDenied("Access is denied".into()),
            WindowsError::Unsupported("no Windows stack here".into()),
            WindowsError::InvalidTarget("nope".into()),
            WindowsError::Launch("could not start".into()),
            WindowsError::Unreadable("empty".into()),
            WindowsError::Remote("RPC server unavailable".into()),
        ] {
            let reason = error.reason();
            assert!(!reason.trim().is_empty());
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_non_windows_build_fails_clearly_rather_than_pretending() {
        let store = CredentialStore::new();
        store.set(WindowsCredential::new("admin", None, "pw").unwrap());
        let target = ProbeTarget::new(Ipv4Addr::new(10, 0, 0, 5), None);
        let result = probe(&target, Transport::Http, &store, || false).await;
        match result {
            Err(WindowsError::Unsupported(reason)) => {
                assert!(reason.contains("Windows build"));
            }
            other => panic!("expected a clear refusal, got {other:?}"),
        }
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_probe_that_is_cancelled_before_it_starts_says_so() {
        // The cancellation path is checked on every platform, because it is
        // the one that decides whether a credential leaves the machine at all.
        let store = CredentialStore::new();
        store.set(WindowsCredential::new("admin", None, "pw").unwrap());
        let target = ProbeTarget::new(Ipv4Addr::new(10, 0, 0, 5), None);
        let result = probe(&target, Transport::Http, &store, || true).await;
        assert!(matches!(result, Err(WindowsError::Cancelled)));
    }

    #[test]
    fn a_probe_target_prefers_a_validated_host_name() {
        // Kerberos authenticates against an SPN built from the host name.
        let named = ProbeTarget::new(Ipv4Addr::new(10, 0, 0, 5), Some("APP-01.corp.example"));
        assert_eq!(named.connect_name(), "APP-01.corp.example");
        assert!(named.is_kerberos_friendly());
    }

    #[test]
    fn a_probe_target_falls_back_to_the_address_when_there_is_no_name() {
        let bare = ProbeTarget::new(Ipv4Addr::new(10, 0, 0, 5), None);
        assert_eq!(bare.connect_name(), "10.0.0.5");
        assert!(!bare.is_kerberos_friendly());
    }

    #[test]
    fn a_hostname_that_could_reach_a_shell_is_discarded_not_used() {
        // Reverse DNS is attacker-influenced on a network ArcScan does not
        // control, and the name is interpolated into a PowerShell script.
        for hostile in [
            "app-01; Start-Process calc",
            "app-01'",
            "$(whoami)",
            "app 01",
            "app`01",
            "-leading-hyphen",
            "trailing-hyphen-",
            "",
            "..",
            "10.0.0.5",
        ] {
            assert_eq!(
                validate_hostname(hostile),
                None,
                "{hostile} must be refused"
            );
            // And a target built with it falls back to the address rather than
            // carrying it forward.
            let target = ProbeTarget::new(Ipv4Addr::new(10, 0, 0, 5), Some(hostile));
            assert_eq!(target.connect_name(), "10.0.0.5");
        }
    }

    #[test]
    fn ordinary_host_names_are_accepted() {
        for good in [
            "APP-01",
            "app-01.corp.example",
            "ws_finance_04",
            "a.b.c.d.example.com",
        ] {
            assert!(validate_hostname(good).is_some(), "{good} should be usable");
        }
        // A trailing root dot is normal in reverse DNS and is trimmed.
        assert_eq!(
            validate_hostname("app-01.corp.example.").as_deref(),
            Some("app-01.corp.example")
        );
    }

    #[test]
    fn dns_length_limits_are_enforced() {
        let long_label = "a".repeat(64);
        assert_eq!(validate_hostname(&long_label), None);
        assert!(validate_hostname(&"a".repeat(63)).is_some());
        let long_name = std::iter::repeat_n("abcdefgh", 40)
            .collect::<Vec<_>>()
            .join(".");
        assert!(long_name.len() > 253);
        assert_eq!(validate_hostname(&long_name), None);
    }

    #[test]
    fn a_missing_listener_is_a_skip_rather_than_a_failure() {
        // A workstation with remote management off is the ordinary case.
        // Counting it as a failure would bury a genuinely refused credential
        // among every desktop on the site.
        assert!(WindowsError::NoManagementTransport.is_skip());
        assert!(WindowsError::Cancelled.is_skip());
        assert!(WindowsError::NoCredential.is_skip());
        assert!(!WindowsError::AccessDenied("denied".into()).is_skip());
        assert!(!WindowsError::Timeout.is_skip());
    }

    #[test]
    fn a_certificate_problem_is_reported_as_one() {
        // So the operator knows the remedy is a trusted certificate rather
        // than a different password.
        for message in [
            "The SSL certificate is signed by an unknown certificate authority",
            "The SSL connection cannot be established",
            "CN name does not match the passed value",
        ] {
            assert!(
                matches!(
                    classify_failure(message),
                    WindowsError::CertificateRejected(_)
                ),
                "{message}"
            );
        }
        // And it is not mistaken for a bad password, which would send an
        // operator to change a credential that was never the problem.
        assert!(!classify_failure(
            "The SSL certificate is signed by an unknown certificate authority"
        )
        .is_skip());
    }

    #[cfg(not(windows))]
    #[test]
    fn the_credential_status_reports_the_platform_as_unsupported() {
        let store = CredentialStore::new();
        let status = store.status();
        assert!(!status.supported);
        assert!(status.unsupported_reason.is_some());
    }
}
