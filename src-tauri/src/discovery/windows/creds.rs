//! Windows credentials, held for the life of the process and nowhere else.
//!
//! # The whole security posture, in one place
//!
//! A credentialed scan needs a password in memory for as long as it takes to
//! ask a machine about itself. Everything beyond that is a liability, so this
//! module is built around what must *not* happen:
//!
//! * **No persistence.** Nothing here touches SQLite, the keyring, a config
//!   file or `localStorage`. A restart is a fresh start, every time. ArcScan
//!   already has a keyring dependency for the ArcAtlas token; it is
//!   deliberately not used here.
//! * **No logging.** [`Secret`] has a hand-written [`Debug`] that prints a
//!   placeholder, so a password cannot reach a log line, a panic message or a
//!   diagnostic report even by accident — `{:?}` on the whole credential is
//!   safe.
//! * **No serialization.** Neither [`Secret`] nor [`WindowsCredential`] derives
//!   `Serialize`, so neither can cross the IPC boundary, enter an export or be
//!   put in a URL. What the frontend may see is [`CredentialStatus`], which
//!   carries a user name and nothing else.
//! * **No guessing.** There is one credential, and the operator typed it. This
//!   module offers no way to try a second one, so password spraying is not a
//!   policy here — it is absent from the code.
//! * **Erased on drop.** The bytes are overwritten through a volatile write
//!   before the allocation is released, so a freed heap page does not keep the
//!   password readable.

use std::fmt;
use std::sync::Mutex;

/// A byte string that is erased when it goes out of scope and never printed.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(value: &str) -> Self {
        Secret(value.as_bytes().to_vec())
    }

    /// The only way to read the secret back.
    ///
    /// Named to be conspicuous at the call site: a reviewer scanning for where
    /// a password could escape only has to look at uses of this method, and
    /// there is exactly one, in the process launcher that writes it to a child
    /// process's stdin.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // A volatile write cannot be optimised away as a dead store, which an
        // ordinary loop over a buffer about to be freed certainly would be.
        for byte in self.0.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Deliberately not the length either: that is information about the
        // password, and a diagnostic report has no use for it.
        f.write_str("Secret(<redacted>)")
    }
}

/// How an account name was written, and how it must be written back.
///
/// The three forms are *not* interchangeable, which is the whole reason this
/// is an enum rather than an `Option<String>` domain field.
///
/// `DOMAIN\\user` is a down-level logon name and `DOMAIN` is the NetBIOS
/// domain name: at most 15 characters, and it cannot contain a dot.
/// `user@corp.example` is a user principal name and the suffix is a DNS name,
/// which is frequently *not* equal to the NetBIOS name — a forest routinely
/// has `CORP` and `corp.example.com`, and can have UPN suffixes that
/// correspond to no domain name at all.
///
/// Rewriting one form as the other is therefore not a formatting choice; it
/// produces an account name that may not authenticate. v1.9.0 did exactly
/// that, turning `user@corp.example` into `corp.example\\user`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountForm {
    /// `user`. A local account on the target machine.
    Local,
    /// `DOMAIN\\user`. The NetBIOS domain name.
    NetBios(String),
    /// `user@suffix`. A user principal name with a DNS-style suffix.
    Upn(String),
}

/// What an operator typed in the credentialed-scan dialog.
#[derive(Debug)]
pub struct WindowsCredential {
    /// The user name alone, without a domain prefix or a UPN suffix.
    pub username: String,
    /// Which of the three forms the operator supplied.
    pub form: AccountForm,
    pub password: Secret,
}

impl WindowsCredential {
    /// Build a credential, rejecting the empty inputs that would otherwise turn
    /// a credentialed scan into an anonymous one that reports as authenticated.
    pub fn new(username: &str, domain: Option<&str>, password: &str) -> Result<Self, String> {
        let username = username.trim();
        if username.is_empty() {
            return Err("A user name is required.".into());
        }
        if password.is_empty() {
            return Err("A password is required.".into());
        }
        // `DOMAIN\user` and `user@domain` are both what a technician has in
        // their notes, so both are accepted and split here rather than being
        // refused for being the wrong shape — and, since v1.9.1, each is
        // written back in the form it arrived in.
        let (form, username) = split_account(username, domain);
        Ok(WindowsCredential {
            username,
            form,
            password: Secret::new(password),
        })
    }

    /// The account name exactly as it must be presented to Windows.
    ///
    /// Round-trips the form the operator supplied: a UPN stays a UPN, a
    /// down-level name stays down-level, a local account stays bare. This
    /// string is what reaches `New-Object PSCredential`, so a rewrite here is
    /// an authentication failure on the remote machine.
    ///
    /// Not a secret: it is what the status line shows and what the target logs
    /// as the connecting account.
    pub fn account(&self) -> String {
        match &self.form {
            AccountForm::Local => self.username.clone(),
            AccountForm::NetBios(domain) => format!("{domain}\\{}", self.username),
            AccountForm::Upn(suffix) => format!("{}@{suffix}", self.username),
        }
    }
}

/// Split an account name into its form and its bare user name.
///
/// An explicitly supplied domain wins over one embedded in the user name,
/// because a separate Domain field is an operator saying which domain they
/// mean. Its *form* is then decided by whether it looks like a DNS name: a
/// NetBIOS domain name cannot contain a dot, so a dotted value is a UPN
/// suffix. That is a rule about the naming scheme rather than a guess.
fn split_account(username: &str, domain: Option<&str>) -> (AccountForm, String) {
    let explicit = domain
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(form_for_domain);

    if let Some((left, right)) = username.split_once('\\') {
        let left = left.trim();
        let right = right.trim();
        if !left.is_empty() && !right.is_empty() {
            return (
                explicit.unwrap_or_else(|| AccountForm::NetBios(left.to_string())),
                right.to_string(),
            );
        }
    }
    if let Some((left, right)) = username.split_once('@') {
        let left = left.trim();
        let right = right.trim();
        if !left.is_empty() && !right.is_empty() {
            // Preserved as a UPN. Rewriting it down-level would substitute a
            // DNS suffix for a NetBIOS name, which are not the same thing.
            return (
                explicit.unwrap_or_else(|| AccountForm::Upn(right.to_string())),
                left.to_string(),
            );
        }
    }
    (explicit.unwrap_or(AccountForm::Local), username.to_string())
}

/// A dotted domain is a DNS suffix and belongs in a UPN; an undotted one is a
/// NetBIOS name, which by definition cannot contain a dot.
fn form_for_domain(domain: &str) -> AccountForm {
    if domain.contains('.') {
        AccountForm::Upn(domain.to_string())
    } else {
        AccountForm::NetBios(domain.to_string())
    }
}

/// What the interface is allowed to know about the stored credential.
///
/// Carries no password and no way to get one. This is the only credential-
/// shaped thing in ArcScan that is `Serialize`, and that is the point.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CredentialStatus {
    pub configured: bool,
    /// `DOMAIN\user`, for a status line that says which account will be used.
    pub account: Option<String>,
    /// True on builds where a credentialed scan can actually run.
    pub supported: bool,
    /// Why not, when `supported` is false. Plain words, shown as-is.
    pub unsupported_reason: Option<String>,
}

/// The process-wide credential, for this run of ArcScan only.
#[derive(Default)]
pub struct CredentialStore {
    inner: Mutex<Option<WindowsCredential>>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, credential: WindowsCredential) {
        // The previous credential is dropped here, which erases it.
        *self.lock() = Some(credential);
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    pub fn status(&self) -> CredentialStatus {
        let guard = self.lock();
        let support = super::platform_support();
        CredentialStatus {
            configured: guard.is_some(),
            account: guard.as_ref().map(WindowsCredential::account),
            supported: support.is_ok(),
            unsupported_reason: support.err(),
        }
    }

    /// Run `f` with the credential, or return `None` when none is set.
    ///
    /// Borrowed under the lock rather than cloned, so there is never a second
    /// copy of the password with its own lifetime to reason about.
    pub fn with<T>(&self, f: impl FnOnce(&WindowsCredential) -> T) -> Option<T> {
        self.lock().as_ref().map(f)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<WindowsCredential>> {
        // A panic while holding this lock would poison it and disable
        // credentialed scanning for the rest of the session; recovering the
        // guard keeps that from turning one bug into a dead feature.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints_itself() {
        let secret = Secret::new("hunter2");
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("hunter2"));
        assert_eq!(rendered, "Secret(<redacted>)");
    }

    #[test]
    fn debugging_a_whole_credential_cannot_leak_the_password() {
        let credential = WindowsCredential::new("admin", Some("CORP"), "hunter2").unwrap();
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("admin"));
    }

    #[test]
    fn a_down_level_account_stays_down_level() {
        let credential = WindowsCredential::new("CORP\\admin", None, "pw").unwrap();
        assert_eq!(credential.form, AccountForm::NetBios("CORP".into()));
        assert_eq!(credential.username, "admin");
        assert_eq!(credential.account(), "CORP\\admin");
    }

    #[test]
    fn a_upn_stays_a_upn() {
        // The v1.9.0 regression: this used to come back as
        // `corp.example\admin`, substituting a DNS suffix for a NetBIOS name.
        // They are different namespaces and the rewrite does not authenticate.
        let credential = WindowsCredential::new("admin@corp.example", None, "pw").unwrap();
        assert_eq!(credential.form, AccountForm::Upn("corp.example".into()));
        assert_eq!(credential.username, "admin");
        assert_eq!(credential.account(), "admin@corp.example");
        assert!(!credential.account().contains('\\'));
    }

    #[test]
    fn a_upn_with_a_multi_label_suffix_is_preserved_whole() {
        let credential =
            WindowsCredential::new("svc-arcscan@ad.corp.example.com", None, "pw").unwrap();
        assert_eq!(credential.account(), "svc-arcscan@ad.corp.example.com");
    }

    #[test]
    fn an_explicit_domain_wins_over_one_embedded_in_the_user_name() {
        let credential = WindowsCredential::new("OLD\\admin", Some("NEW"), "pw").unwrap();
        assert_eq!(credential.form, AccountForm::NetBios("NEW".into()));
        assert_eq!(credential.username, "admin");
        assert_eq!(credential.account(), "NEW\\admin");
    }

    #[test]
    fn an_explicit_dotted_domain_is_read_as_a_upn_suffix() {
        // A NetBIOS domain name cannot contain a dot, so a dotted value in the
        // Domain field is a DNS suffix. Writing `corp.example\admin` would be
        // the same substitution this release exists to stop.
        let credential = WindowsCredential::new("admin", Some("corp.example"), "pw").unwrap();
        assert_eq!(credential.form, AccountForm::Upn("corp.example".into()));
        assert_eq!(credential.account(), "admin@corp.example");
    }

    #[test]
    fn an_explicit_domain_also_overrides_a_supplied_upn_suffix() {
        let credential = WindowsCredential::new("admin@old.example", Some("NEW"), "pw").unwrap();
        assert_eq!(credential.account(), "NEW\\admin");
    }

    #[test]
    fn a_local_account_stays_bare() {
        let credential = WindowsCredential::new("Administrator", None, "pw").unwrap();
        assert_eq!(credential.form, AccountForm::Local);
        assert_eq!(credential.account(), "Administrator");
    }

    #[test]
    fn every_account_form_round_trips_through_account() {
        // The property that matters: whatever an operator typed is what the
        // remote machine is asked to authenticate.
        for supplied in [
            "Administrator",
            "CORP\\admin",
            "admin@corp.example",
            "svc-arcscan@ad.corp.example.com",
            "WORKGROUP\\guest",
        ] {
            let credential = WindowsCredential::new(supplied, None, "pw").unwrap();
            assert_eq!(
                credential.account(),
                supplied,
                "{supplied} was rewritten as {}",
                credential.account()
            );
        }
    }

    #[test]
    fn empty_inputs_are_refused_rather_than_scanning_anonymously() {
        assert!(WindowsCredential::new("", None, "pw").is_err());
        assert!(WindowsCredential::new("  ", None, "pw").is_err());
        assert!(WindowsCredential::new("admin", None, "").is_err());
    }

    #[test]
    fn the_store_reports_the_account_but_never_the_password() {
        let store = CredentialStore::new();
        assert!(!store.status().configured);
        assert_eq!(store.status().account, None);

        store.set(WindowsCredential::new("CORP\\admin", None, "hunter2").unwrap());
        let status = store.status();
        assert!(status.configured);
        assert_eq!(status.account.as_deref(), Some("CORP\\admin"));
        // The status is the only credential-shaped thing that serializes, so
        // this is the test that the password cannot reach the frontend.
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("hunter2"));
        assert!(!json.to_lowercase().contains("password"));

        store.clear();
        assert!(!store.status().configured);
    }

    #[test]
    fn setting_a_second_credential_replaces_the_first() {
        let store = CredentialStore::new();
        store.set(WindowsCredential::new("one", None, "pw1").unwrap());
        store.set(WindowsCredential::new("two", None, "pw2").unwrap());
        assert_eq!(store.status().account.as_deref(), Some("two"));
    }

    #[test]
    fn the_credential_is_readable_only_through_with() {
        let store = CredentialStore::new();
        assert!(store.with(|c| c.account()).is_none());
        store.set(WindowsCredential::new("admin", None, "pw").unwrap());
        assert_eq!(store.with(|c| c.account()).as_deref(), Some("admin"));
        assert_eq!(
            store.with(|c| c.password.expose().to_vec()),
            Some(b"pw".to_vec())
        );
    }
}
