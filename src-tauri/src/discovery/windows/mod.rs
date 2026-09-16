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

pub use creds::{CredentialStatus, CredentialStore, WindowsCredential};
pub use facts::{WindowsFacts, WindowsProductType};

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
    if lower.contains("timed out") || lower.contains("timeout") {
        return WindowsError::Timeout;
    }
    WindowsError::Remote(message.trim().to_string())
}

/// Ask one Windows machine about itself.
///
/// The credential is borrowed under the store's lock for exactly as long as it
/// takes to write it to the child process's stdin, and is never copied into an
/// argument, an environment variable, a file or a log line.
#[cfg(windows)]
pub async fn probe(target: &str, store: &CredentialStore) -> Result<WindowsFacts, WindowsError> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let target = validate_target(target)?;

    // The account and the password are taken together, under one lock, so the
    // pair cannot change between building the command and writing the secret.
    let Some((account, password)) =
        store.with(|credential| (credential.account(), credential.password.expose().to_vec()))
    else {
        return Err(WindowsError::NoCredential);
    };

    let encoded = script::encode_command(&script::collection_script(&target, &account));

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
        .spawn()
        .map_err(|e| WindowsError::Launch(e.to_string()))?;

    // stdin is the only place the password goes. The buffer is zeroed
    // immediately afterwards rather than waiting to be dropped.
    let mut password = password;
    if let Some(mut stdin) = child.stdin.take() {
        let write = async {
            stdin.write_all(&password).await?;
            stdin.write_all(b"\r\n").await?;
            stdin.flush().await?;
            stdin.shutdown().await
        }
        .await;
        for byte in password.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        write.map_err(|e| WindowsError::Launch(e.to_string()))?;
    } else {
        for byte in password.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        return Err(WindowsError::Launch(
            "the management query would not accept input".into(),
        ));
    }

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(PROBE_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Err(_) => return Err(WindowsError::Timeout),
        Ok(Err(e)) => return Err(WindowsError::Launch(e.to_string())),
        Ok(Ok(output)) => output,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
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

/// The non-Windows answer: a clear refusal.
///
/// Not an empty `WindowsFacts`, and not a silent skip. A technician running the
/// macOS or Linux build who asks for a credentialed scan is told why it did not
/// happen.
#[cfg(not(windows))]
pub async fn probe(target: &str, _store: &CredentialStore) -> Result<WindowsFacts, WindowsError> {
    // Validated first so the refusal for a malformed address is the same on
    // every platform, which keeps the tests honest.
    validate_target(target)?;
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
        let result = probe("10.0.0.5", &store).await;
        match result {
            Err(WindowsError::Unsupported(reason)) => {
                assert!(reason.contains("Windows build"));
            }
            other => panic!("expected a clear refusal, got {other:?}"),
        }
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
