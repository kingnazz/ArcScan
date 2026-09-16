//! SNMP session abstraction.
//!
//! SNMPv2c is implemented here on Tokio UDP so timeouts, retries and
//! cancellation stay under ArcScan control. SNMPv3 is delegated to `snmp2`
//! (USM with auth and privacy). The correlator never talks to the wire: it
//! consumes a [`SnmpSession`], which tests replace with a fixture table.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::ber::{
    decode_message, encode_get, encode_get_bulk, Oid, SnmpValue, VarBind, PDU_GET_BULK,
};
use super::credentials::{AuthProtocol, PrivProtocol, SnmpSecret, SnmpVersion};
use super::error::TopologyError;

/// How many GETBULK rounds a single walk may run.
pub const MAX_WALK_ROUNDS: usize = 80;
/// How many variable-bindings a walk may keep.
pub const MAX_WALK_BINDS: usize = 4_096;
/// GETBULK max-repetitions. Conservative so a broken agent cannot flood us.
pub const MAX_REPETITIONS: i32 = 20;

pub trait SnmpSession: Send + Sync {
    fn get<'a>(
        &'a self,
        oids: &'a [Oid],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    >;

    fn walk<'a>(
        &'a self,
        root: &'a Oid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    >;
}

/// In-memory SNMP table keyed by dotted OID. Deterministic, no network.
pub struct FixtureSession {
    pub values: BTreeMap<String, SnmpValue>,
    pub fail: Option<TopologyError>,
}

impl FixtureSession {
    pub fn new(values: BTreeMap<String, SnmpValue>) -> Self {
        Self { values, fail: None }
    }

    pub fn failing(err: TopologyError) -> Self {
        Self {
            values: BTreeMap::new(),
            fail: Some(err),
        }
    }
}

impl SnmpSession for FixtureSession {
    fn get<'a>(
        &'a self,
        oids: &'a [Oid],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if let Some(err) = &self.fail {
                return Err(err.clone());
            }
            let mut out = Vec::new();
            for oid in oids {
                let key = oid.to_dotted();
                let value = self
                    .values
                    .get(&key)
                    .cloned()
                    .unwrap_or(SnmpValue::NoSuchInstance);
                out.push(VarBind {
                    oid: oid.clone(),
                    value,
                });
            }
            Ok(out)
        })
    }

    fn walk<'a>(
        &'a self,
        root: &'a Oid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if let Some(err) = &self.fail {
                return Err(err.clone());
            }
            let prefix = root.to_dotted();
            let mut out: Vec<VarBind> = self
                .values
                .iter()
                .filter(|(k, _)| k == &&prefix || k.starts_with(&format!("{prefix}.")))
                .filter_map(|(k, v)| {
                    Oid::parse(k).ok().map(|oid| VarBind {
                        oid,
                        value: v.clone(),
                    })
                })
                .collect();
            out.sort_by(|a, b| a.oid.0.cmp(&b.oid.0));
            Ok(out)
        })
    }
}

pub struct V2cSession {
    addr: SocketAddr,
    community: Vec<u8>,
    timeout: Duration,
    socket: UdpSocket,
    request_id: AtomicI32,
}

impl V2cSession {
    pub async fn connect(
        ip: Ipv4Addr,
        community: Vec<u8>,
        timeout: Duration,
    ) -> Result<Self, TopologyError> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|_| TopologyError::Unreachable)?;
        Ok(Self {
            addr: SocketAddr::from((ip, 161)),
            community,
            timeout,
            socket,
            request_id: AtomicI32::new(1),
        })
    }

    fn next_id(&self) -> i32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn transact(
        &self,
        payload: Vec<u8>,
        expect_id: i32,
    ) -> Result<Vec<VarBind>, TopologyError> {
        let mut last_err = TopologyError::Timeout;
        // One retry. A lost UDP packet is common; a third attempt would
        // stretch a dead agent across the whole site budget.
        for _ in 0..2 {
            if self.socket.send_to(&payload, self.addr).await.is_err() {
                return Err(TopologyError::Unreachable);
            }
            let mut buf = vec![0u8; 8_192];
            match timeout(self.timeout, self.socket.recv_from(&mut buf)).await {
                Err(_) => {
                    last_err = TopologyError::Timeout;
                    continue;
                }
                Ok(Err(_)) => return Err(TopologyError::Unreachable),
                Ok(Ok((n, from))) => {
                    if from.ip() != self.addr.ip() {
                        last_err = TopologyError::Protocol(
                            "SNMP reply came from an unexpected address.".into(),
                        );
                        continue;
                    }
                    let msg = decode_message(&buf[..n]).map_err(|_| {
                        TopologyError::Protocol(
                            "The device sent an SNMP packet ArcScan could not read.".into(),
                        )
                    })?;
                    if msg.pdu.request_id != expect_id {
                        last_err =
                            TopologyError::Protocol("SNMP reply request-id did not match.".into());
                        continue;
                    }
                    // error-status 16 is authorizationError (SNMPv2). Wrong
                    // community more often produces silence, which is Timeout.
                    if msg.pdu.error_status == 16 {
                        return Err(TopologyError::AuthFailed);
                    }
                    if msg.pdu.error_status != 0 && msg.pdu.tag != PDU_GET_BULK {
                        // GETBULK reuses error-status as non-repeaters on the
                        // request; a response with a non-zero status is a real
                        // error, but we still return whatever binds arrived so
                        // a partially implemented MIB is not discarded.
                        if msg.pdu.binds.is_empty() {
                            return Err(TopologyError::Protocol(
                                "The SNMP agent reported an error for this request.".into(),
                            ));
                        }
                    }
                    return Ok(msg.pdu.binds);
                }
            }
        }
        Err(last_err)
    }

    async fn get_inner(&self, oids: &[Oid]) -> Result<Vec<VarBind>, TopologyError> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let id = self.next_id();
        let payload = encode_get(&self.community, id, oids);
        self.transact(payload, id).await
    }

    async fn walk_inner(&self, root: &Oid) -> Result<Vec<VarBind>, TopologyError> {
        let mut cursor = root.clone();
        let mut out = Vec::new();
        for _ in 0..MAX_WALK_ROUNDS {
            let id = self.next_id();
            let payload =
                encode_get_bulk(&self.community, id, 0, MAX_REPETITIONS, &[cursor.clone()]);
            let binds = self.transact(payload, id).await?;
            if binds.is_empty() {
                break;
            }
            let mut progressed = false;
            for bind in binds {
                if !bind.oid.starts_with(&root.0) || bind.value.is_end() {
                    return Ok(out);
                }
                cursor = bind.oid.clone();
                out.push(bind);
                progressed = true;
                if out.len() >= MAX_WALK_BINDS {
                    return Ok(out);
                }
            }
            if !progressed {
                break;
            }
        }
        Ok(out)
    }
}

impl SnmpSession for V2cSession {
    fn get<'a>(
        &'a self,
        oids: &'a [Oid],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(self.get_inner(oids))
    }

    fn walk<'a>(
        &'a self,
        root: &'a Oid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(self.walk_inner(root))
    }
}

/// SNMPv3 session. Blocking `snmp2` calls run on the blocking pool so the
/// async scanner is not stalled, and every error is rewritten so a library
/// message cannot echo a password.
pub struct V3Session {
    ip: Ipv4Addr,
    timeout: Duration,
    secret: SnmpSecret,
    inner: std::sync::Arc<std::sync::Mutex<Option<snmp2::SyncSession>>>,
}

impl V3Session {
    pub fn new(ip: Ipv4Addr, timeout: Duration, secret: SnmpSecret) -> Self {
        Self {
            ip,
            timeout,
            secret,
            inner: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn build_security(&self) -> Result<snmp2::v3::Security, TopologyError> {
        let SnmpSecret::V3 {
            username,
            auth_protocol,
            auth_password,
            priv_protocol,
            priv_password,
            context,
        } = &self.secret
        else {
            return Err(TopologyError::Internal(
                "SNMPv3 session constructed without v3 credentials.".into(),
            ));
        };
        let auth_password = auth_password.as_deref().unwrap_or(&[]);
        let mut security = snmp2::v3::Security::new(username.as_bytes(), auth_password);
        if let Some(proto) = auth_protocol {
            security = security.with_auth_protocol(map_auth(*proto));
        }
        if let (Some(proto), Some(pass)) = (priv_protocol, priv_password) {
            security = security.with_auth(snmp2::v3::Auth::AuthPriv {
                cipher: map_cipher(*proto),
                privacy_password: pass.clone(),
            });
        } else {
            security = security.with_auth(snmp2::v3::Auth::AuthNoPriv);
        }
        if let Some(ctx) = context {
            security = security.with_context_name(ctx);
        }
        Ok(security)
    }

    async fn with_session<F, T>(&self, op: F) -> Result<T, TopologyError>
    where
        F: FnOnce(&mut snmp2::SyncSession) -> Result<T, TopologyError> + Send + 'static,
        T: Send + 'static,
    {
        let timeout = self.timeout;
        let ip = self.ip;
        let security = self.build_security()?;
        let inner = std::sync::Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let mut guard = inner
                .lock()
                .map_err(|_| TopologyError::Internal("SNMPv3 session lock was poisoned.".into()))?;
            if guard.is_none() {
                let addr = format!("{ip}:161");
                let mut sess = snmp2::SyncSession::new_v3(&addr, Some(timeout), 1, security)
                    .map_err(|_| TopologyError::Unreachable)?;
                // Engine-id discovery. Failure here is a timeout or auth problem,
                // not a reason to panic.
                sess.init().map_err(map_snmp2_error)?;
                *guard = Some(sess);
            }
            let sess = guard.as_mut().expect("session just inserted");
            op(sess)
        })
        .await
        .map_err(|_| TopologyError::Internal("SNMPv3 worker stopped unexpectedly.".into()))?
    }
}

fn map_auth(proto: AuthProtocol) -> snmp2::v3::AuthProtocol {
    match proto {
        AuthProtocol::Md5 => snmp2::v3::AuthProtocol::Md5,
        AuthProtocol::Sha1 => snmp2::v3::AuthProtocol::Sha1,
        AuthProtocol::Sha224 => snmp2::v3::AuthProtocol::Sha224,
        AuthProtocol::Sha256 => snmp2::v3::AuthProtocol::Sha256,
        AuthProtocol::Sha384 => snmp2::v3::AuthProtocol::Sha384,
        AuthProtocol::Sha512 => snmp2::v3::AuthProtocol::Sha512,
    }
}

fn map_cipher(proto: PrivProtocol) -> snmp2::v3::Cipher {
    match proto {
        PrivProtocol::Des => snmp2::v3::Cipher::Des,
        PrivProtocol::Aes128 => snmp2::v3::Cipher::Aes128,
        PrivProtocol::Aes192 => snmp2::v3::Cipher::Aes192,
        PrivProtocol::Aes256 => snmp2::v3::Cipher::Aes256,
    }
}

fn map_snmp2_error(err: snmp2::Error) -> TopologyError {
    // Convert to a display string, then drop anything that looks like a secret.
    // We never return the library's raw Display to the UI.
    let raw = format!("{err:?}");
    let lower = raw.to_ascii_lowercase();
    if lower.contains("timeout") {
        TopologyError::Timeout
    } else if lower.contains("auth") || lower.contains("decrypt") || lower.contains("security") {
        TopologyError::AuthFailed
    } else if lower.contains("io") || lower.contains("unreach") {
        TopologyError::Unreachable
    } else {
        TopologyError::Protocol("The SNMPv3 agent rejected the request.".into())
    }
}

fn oid_to_snmp2(arcs: &[u32]) -> Result<snmp2::Oid<'static>, TopologyError> {
    let u64s: Vec<u64> = arcs.iter().map(|n| u64::from(*n)).collect();
    snmp2::Oid::from(&u64s).map_err(|_| TopologyError::Protocol("OID could not be encoded.".into()))
}

fn value_from_snmp2(value: snmp2::Value<'_>) -> SnmpValue {
    match value {
        snmp2::Value::Integer(v) => SnmpValue::Integer(v),
        snmp2::Value::OctetString(v) => SnmpValue::OctetString(v.to_vec()),
        snmp2::Value::ObjectIdentifier(oid) => {
            let dotted = oid.to_string();
            Oid::parse(dotted.trim_start_matches('.'))
                .map(SnmpValue::Oid)
                .unwrap_or(SnmpValue::Null)
        }
        snmp2::Value::Null => SnmpValue::Null,
        snmp2::Value::IpAddress(octets) => {
            SnmpValue::IpAddress(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
        }
        snmp2::Value::Counter32(v) => SnmpValue::Counter32(v),
        snmp2::Value::Unsigned32(v) => SnmpValue::Gauge32(v),
        snmp2::Value::Timeticks(v) => SnmpValue::TimeTicks(v),
        snmp2::Value::Counter64(v) => SnmpValue::Counter64(v),
        snmp2::Value::EndOfMibView => SnmpValue::EndOfMibView,
        snmp2::Value::NoSuchObject => SnmpValue::NoSuchObject,
        snmp2::Value::NoSuchInstance => SnmpValue::NoSuchInstance,
        other => {
            let _ = other;
            SnmpValue::Null
        }
    }
}

fn collect_varbinds(response: snmp2::Pdu<'_>) -> Vec<VarBind> {
    let mut out = Vec::new();
    for (oid, value) in response.varbinds {
        let dotted = oid.to_string();
        if let Ok(parsed) = Oid::parse(dotted.trim_start_matches('.')) {
            out.push(VarBind {
                oid: parsed,
                value: value_from_snmp2(value),
            });
        }
    }
    out
}

fn retry_auth(
    sess: &mut snmp2::SyncSession,
    mut op: impl for<'a> FnMut(&'a mut snmp2::SyncSession) -> Result<snmp2::Pdu<'a>, snmp2::Error>,
) -> Result<Vec<VarBind>, TopologyError> {
    match op(sess) {
        Ok(v) => Ok(collect_varbinds(v)),
        Err(snmp2::Error::AuthUpdated) => {
            sess.init().map_err(map_snmp2_error)?;
            op(sess).map(collect_varbinds).map_err(map_snmp2_error)
        }
        Err(e) => Err(map_snmp2_error(e)),
    }
}

impl V3Session {
    async fn get_inner(&self, oids: &[Oid]) -> Result<Vec<VarBind>, TopologyError> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let owned: Vec<Vec<u32>> = oids.iter().map(|o| o.0.clone()).collect();
        self.with_session(move |sess| {
            if owned.len() == 1 {
                let oid = oid_to_snmp2(&owned[0])?;
                retry_auth(sess, |sess| sess.get(&oid))
            } else {
                let parsed: Vec<snmp2::Oid> = owned
                    .iter()
                    .map(|arcs| oid_to_snmp2(arcs))
                    .collect::<Result<Vec<_>, _>>()?;
                retry_auth(sess, |sess| {
                    let refs: Vec<&snmp2::Oid> = parsed.iter().collect();
                    sess.get_many(&refs)
                })
            }
        })
        .await
    }

    async fn walk_inner(&self, root: &Oid) -> Result<Vec<VarBind>, TopologyError> {
        let prefix = root.0.clone();
        self.with_session(move |sess| {
            let mut cursor_arcs = prefix.clone();
            let mut out = Vec::new();
            for _ in 0..MAX_WALK_ROUNDS {
                let cursor = oid_to_snmp2(&cursor_arcs)?;
                let response = retry_auth(sess, |sess| {
                    let refs = [&cursor];
                    sess.getbulk(&refs, 0, MAX_REPETITIONS as u32)
                })?;
                let binds = response;
                let mut progressed = false;
                let mut stop = false;
                for bind in binds {
                    if !bind.oid.starts_with(&prefix) || bind.value.is_end() {
                        stop = true;
                        break;
                    }
                    cursor_arcs = bind.oid.0.clone();
                    out.push(bind);
                    progressed = true;
                    if out.len() >= MAX_WALK_BINDS {
                        stop = true;
                        break;
                    }
                }
                if stop || !progressed {
                    break;
                }
            }
            Ok(out)
        })
        .await
    }
}

impl SnmpSession for V3Session {
    fn get<'a>(
        &'a self,
        oids: &'a [Oid],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(self.get_inner(oids))
    }

    fn walk<'a>(
        &'a self,
        root: &'a Oid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<VarBind>, TopologyError>> + Send + 'a>,
    > {
        Box::pin(self.walk_inner(root))
    }
}

pub async fn open_session(
    ip: Ipv4Addr,
    secret: &SnmpSecret,
    timeout: Duration,
) -> Result<Box<dyn SnmpSession>, TopologyError> {
    match secret.version() {
        SnmpVersion::V2c => {
            let SnmpSecret::V2c { community } = secret else {
                return Err(TopologyError::Internal("v2c secret mismatch".into()));
            };
            let sess = V2cSession::connect(ip, community.clone(), timeout).await?;
            Ok(Box::new(sess))
        }
        SnmpVersion::V3 => Ok(Box::new(V3Session::new(ip, timeout, secret.clone()))),
    }
}

/// Probe: GET sysName. Cheap enough to run against every live host.
pub async fn probe_sysname(session: &dyn SnmpSession) -> Result<Option<String>, TopologyError> {
    let oid = Oid::from_slice(&[1, 3, 6, 1, 2, 1, 1, 5, 0]);
    let binds = session.get(&[oid]).await?;
    Ok(binds.first().and_then(|b| b.value.as_utf8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fixture_get_and_walk() {
        let mut values = BTreeMap::new();
        values.insert(
            "1.3.6.1.2.1.1.5.0".into(),
            SnmpValue::OctetString(b"core-sw".to_vec()),
        );
        values.insert(
            "1.3.6.1.2.1.2.2.1.2.1".into(),
            SnmpValue::OctetString(b"Gi1/0/1".to_vec()),
        );
        values.insert(
            "1.3.6.1.2.1.2.2.1.2.2".into(),
            SnmpValue::OctetString(b"Gi1/0/2".to_vec()),
        );
        let sess = FixtureSession::new(values);
        let got = sess
            .get(&[Oid::parse("1.3.6.1.2.1.1.5.0").unwrap()])
            .await
            .unwrap();
        assert_eq!(got[0].value.as_utf8().as_deref(), Some("core-sw"));
        let walked = sess
            .walk(&Oid::parse("1.3.6.1.2.1.2.2.1.2").unwrap())
            .await
            .unwrap();
        assert_eq!(walked.len(), 2);
    }

    #[tokio::test]
    async fn fixture_timeout_is_isolated() {
        let sess = FixtureSession::failing(TopologyError::Timeout);
        assert!(matches!(
            probe_sysname(&sess).await,
            Err(TopologyError::Timeout)
        ));
    }

    #[tokio::test]
    async fn v2c_timeout_against_documentation_range() {
        // 203.0.113.0/24 is TEST-NET-3. Nothing should answer; the client
        // must return Timeout, not hang, and the error must not mention a
        // community string.
        let sess = V2cSession::connect(
            Ipv4Addr::new(203, 0, 113, 9),
            b"must-not-leak".to_vec(),
            Duration::from_millis(80),
        )
        .await
        .unwrap();
        let err = sess
            .get(&[Oid::parse("1.3.6.1.2.1.1.5.0").unwrap()])
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            TopologyError::Timeout | TopologyError::Unreachable
        ));
        let msg = err.to_string();
        assert!(!msg.contains("must-not-leak"));
    }
}
