//! SMB2 protocol negotiation, as an identity probe.
//!
//! # What one NEGOTIATE exchange is worth
//!
//! The first message of an SMB2 conversation is unauthenticated by design, and
//! the server's reply carries three things ArcScan can use:
//!
//! * the **dialect** it agreed to, which sets a *floor* on the server's
//!   generation;
//! * a **server GUID**, which Windows persists across reboots and which is
//!   therefore a genuine stable identifier for reconciling one machine seen on
//!   several addresses; and
//! * whether **signing is required**, which is the default on a domain
//!   controller and is worth recording as corroboration.
//!
//! # What it is emphatically not worth
//!
//! A dialect is not a Windows version. SMB 3.1.1 means "Windows 10, Windows
//! Server 2016, or anything newer — or Samba 4.3 or newer, on Linux". Reading
//! it as "Windows 10" would be the precise failure v1.9 exists to remove, so
//! [`Dialect::floor`] is worded as a floor and there is no code path that turns
//! it into an [`EvidenceKind::OsVersion`].
//!
//! The exchange stops after the negotiate response. No session setup is
//! attempted, no credential is offered, and no share is enumerated.

use crate::discovery::model::{Confidence, DiscoverySource, Evidence, EvidenceKind};

/// The SMB2 header is 64 bytes, after a 4-byte NetBIOS session header.
const NETBIOS_HEADER: usize = 4;
const SMB2_HEADER: usize = 64;
const BODY: usize = NETBIOS_HEADER + SMB2_HEADER;

/// Longest reply this will read. A negotiate response is a few hundred bytes.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024;

/// A negotiated SMB2 dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect(pub u16);

impl Dialect {
    /// The protocol version, as the protocol names it.
    pub fn label(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0202 => "SMB 2.0.2",
            0x0210 => "SMB 2.1",
            0x0300 => "SMB 3.0",
            0x0302 => "SMB 3.0.2",
            0x0311 => "SMB 3.1.1",
            0x02FF => "SMB 2 (wildcard)",
            _ => return None,
        })
    }

    /// The *oldest* Windows release that speaks this dialect.
    ///
    /// Deliberately phrased as a floor, and deliberately mentioning that a
    /// non-Windows server can speak it too. This string goes in front of a
    /// technician, and it has to be impossible to read as "this machine runs
    /// Windows 10".
    pub fn floor(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0202 => "Windows Vista / Server 2008 or newer, or Samba",
            0x0210 => "Windows 7 / Server 2008 R2 or newer, or Samba",
            0x0300 => "Windows 8 / Server 2012 or newer, or Samba",
            0x0302 => "Windows 8.1 / Server 2012 R2 or newer, or Samba",
            0x0311 => "Windows 10 / Server 2016 or newer, or Samba",
            _ => return None,
        })
    }
}

/// What the server said in its negotiate response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmbFingerprint {
    pub dialect: Dialect,
    /// The server's persistent GUID, rendered in the usual hyphenated form.
    pub server_guid: Option<String>,
    pub signing_enabled: bool,
    /// True when the server will not talk without signing. The default on a
    /// domain controller.
    pub signing_required: bool,
}

/// Build an SMB2 NEGOTIATE request offering the dialects Windows offers.
///
/// The client GUID is all zeroes on purpose: it is the one field that would
/// otherwise let a scanned host correlate one ArcScan installation across
/// visits, and the negotiate exchange has no use for it.
pub fn negotiate_request() -> Vec<u8> {
    let dialects: [u16; 5] = [0x0202, 0x0210, 0x0300, 0x0302, 0x0311];

    let mut message = Vec::with_capacity(SMB2_HEADER + 36 + dialects.len() * 2);
    // ---- SMB2 header ---------------------------------------------------
    message.extend_from_slice(&[0xFE, b'S', b'M', b'B']); // ProtocolId
    message.extend_from_slice(&64u16.to_le_bytes()); // StructureSize
    message.extend_from_slice(&0u16.to_le_bytes()); // CreditCharge
    message.extend_from_slice(&0u32.to_le_bytes()); // Status
    message.extend_from_slice(&0u16.to_le_bytes()); // Command = NEGOTIATE
    message.extend_from_slice(&1u16.to_le_bytes()); // CreditRequest
    message.extend_from_slice(&0u32.to_le_bytes()); // Flags
    message.extend_from_slice(&0u32.to_le_bytes()); // NextCommand
    message.extend_from_slice(&0u64.to_le_bytes()); // MessageId
    message.extend_from_slice(&0u32.to_le_bytes()); // Reserved
    message.extend_from_slice(&0u32.to_le_bytes()); // TreeId
    message.extend_from_slice(&0u64.to_le_bytes()); // SessionId
    message.extend_from_slice(&[0u8; 16]); // Signature

    // ---- NEGOTIATE request ---------------------------------------------
    message.extend_from_slice(&36u16.to_le_bytes()); // StructureSize
    message.extend_from_slice(&(dialects.len() as u16).to_le_bytes()); // DialectCount
    message.extend_from_slice(&1u16.to_le_bytes()); // SecurityMode: signing enabled
    message.extend_from_slice(&0u16.to_le_bytes()); // Reserved
    message.extend_from_slice(&0u32.to_le_bytes()); // Capabilities
    message.extend_from_slice(&[0u8; 16]); // ClientGuid: deliberately zero
    message.extend_from_slice(&0u64.to_le_bytes()); // ClientStartTime
    for dialect in dialects {
        message.extend_from_slice(&dialect.to_le_bytes());
    }

    // ---- NetBIOS session service framing --------------------------------
    let length = message.len();
    let mut framed = Vec::with_capacity(NETBIOS_HEADER + length);
    framed.push(0x00);
    framed.extend_from_slice(&[
        ((length >> 16) & 0xFF) as u8,
        ((length >> 8) & 0xFF) as u8,
        (length & 0xFF) as u8,
    ]);
    framed.extend_from_slice(&message);
    framed
}

/// Parse a negotiate response.
///
/// Every field is read through a bounds-checked slice, so a truncated or
/// hostile reply yields `None` rather than a panic. This is parsing bytes from
/// an unauthenticated stranger; it is written to be boring.
pub fn parse_negotiate_response(raw: &[u8]) -> Option<SmbFingerprint> {
    if raw.len() < BODY + 64 {
        return None;
    }
    // The SMB2 magic sits immediately after the NetBIOS length.
    if raw.get(NETBIOS_HEADER..NETBIOS_HEADER + 4)? != [0xFE, b'S', b'M', b'B'] {
        return None;
    }
    // Command must be NEGOTIATE (0x0000) and the reply must be a response.
    let command = u16::from_le_bytes(raw.get(NETBIOS_HEADER + 12..NETBIOS_HEADER + 14)?.try_into().ok()?);
    if command != 0 {
        return None;
    }
    // A non-zero status means the server refused rather than negotiated.
    let status = u32::from_le_bytes(raw.get(NETBIOS_HEADER + 8..NETBIOS_HEADER + 12)?.try_into().ok()?);
    if status != 0 {
        return None;
    }

    let structure_size = u16::from_le_bytes(raw.get(BODY..BODY + 2)?.try_into().ok()?);
    if structure_size != 65 {
        return None;
    }
    let security_mode = u16::from_le_bytes(raw.get(BODY + 2..BODY + 4)?.try_into().ok()?);
    let dialect = u16::from_le_bytes(raw.get(BODY + 4..BODY + 6)?.try_into().ok()?);
    let guid_bytes: [u8; 16] = raw.get(BODY + 8..BODY + 24)?.try_into().ok()?;

    Some(SmbFingerprint {
        dialect: Dialect(dialect),
        server_guid: format_guid(&guid_bytes),
        signing_enabled: security_mode & 0x0001 != 0,
        signing_required: security_mode & 0x0002 != 0,
    })
}

/// Render a GUID, refusing the all-zero one that means "not set".
///
/// Windows sends its persistent server GUID; an implementation that has none
/// sends zeroes, and recording that as an identity would merge every such
/// server into one device.
fn format_guid(bytes: &[u8; 16]) -> Option<String> {
    if bytes.iter().all(|b| *b == 0) {
        return None;
    }
    // SMB2 sends the GUID as raw bytes, so this is the straight big-endian
    // rendering rather than the mixed-endian one a Microsoft GUID struct uses.
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Turn a fingerprint into discovery evidence.
pub fn evidence(fingerprint: &SmbFingerprint) -> Vec<Evidence> {
    let mut out = Vec::new();

    if let Some(label) = fingerprint.dialect.label() {
        // A banner, not a version. The floor is spelled out in the value so
        // that wherever this string is shown it carries its own caveat.
        let value = match fingerprint.dialect.floor() {
            Some(floor) => format!("{label} (implies {floor})"),
            None => label.to_string(),
        };
        out.push(Evidence::new(
            DiscoverySource::Smb,
            EvidenceKind::Banner,
            "smb-dialect",
            &value,
            Confidence::Medium,
        ));
    }

    if let Some(guid) = &fingerprint.server_guid {
        // A stable identifier, not a name. Recorded as a protocol identifier so
        // reconciliation can use it and naming cannot.
        out.push(Evidence::new(
            DiscoverySource::Smb,
            EvidenceKind::ProtocolIdentifier,
            "smb_server_guid",
            guid,
            Confidence::High,
        ));
    }

    if fingerprint.signing_required {
        out.push(Evidence::new(
            DiscoverySource::Smb,
            EvidenceKind::Banner,
            "smb-signing",
            "SMB signing required",
            Confidence::Medium,
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic negotiate response for the tests.
    fn response(dialect: u16, guid: [u8; 16], security_mode: u16, status: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&65u16.to_le_bytes()); // StructureSize
        body.extend_from_slice(&security_mode.to_le_bytes());
        body.extend_from_slice(&dialect.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes()); // NegotiateContextCount
        body.extend_from_slice(&guid);
        body.extend_from_slice(&0u32.to_le_bytes()); // Capabilities
        body.extend_from_slice(&[0u8; 12]); // Max sizes
        body.extend_from_slice(&[0u8; 16]); // SystemTime, ServerStartTime
        body.extend_from_slice(&[0u8; 8]); // Security buffer, contexts

        let mut message = Vec::new();
        message.extend_from_slice(&[0xFE, b'S', b'M', b'B']);
        message.extend_from_slice(&64u16.to_le_bytes());
        message.extend_from_slice(&0u16.to_le_bytes());
        message.extend_from_slice(&status.to_le_bytes());
        message.extend_from_slice(&0u16.to_le_bytes()); // Command NEGOTIATE
        message.extend_from_slice(&1u16.to_le_bytes());
        message.extend_from_slice(&1u32.to_le_bytes()); // Flags: response
        message.extend_from_slice(&0u32.to_le_bytes());
        message.extend_from_slice(&0u64.to_le_bytes());
        message.extend_from_slice(&0u32.to_le_bytes());
        message.extend_from_slice(&0u32.to_le_bytes());
        message.extend_from_slice(&0u64.to_le_bytes());
        message.extend_from_slice(&[0u8; 16]);
        message.extend_from_slice(&body);

        let length = message.len();
        let mut framed = vec![
            0x00,
            ((length >> 16) & 0xFF) as u8,
            ((length >> 8) & 0xFF) as u8,
            (length & 0xFF) as u8,
        ];
        framed.extend_from_slice(&message);
        framed
    }

    const GUID: [u8; 16] = [
        0x4c, 0x4c, 0x45, 0x44, 0x00, 0x37, 0x5a, 0x10, 0x80, 0x51, 0xb4, 0xc0, 0x4f, 0x43, 0x53,
        0x31,
    ];

    #[test]
    fn the_request_is_framed_and_offers_the_usual_dialects() {
        let request = negotiate_request();
        assert_eq!(request[0], 0x00);
        let declared = ((request[1] as usize) << 16) | ((request[2] as usize) << 8) | request[3] as usize;
        assert_eq!(declared, request.len() - 4);
        assert_eq!(&request[4..8], &[0xFE, b'S', b'M', b'B']);
    }

    #[test]
    fn the_request_sends_a_zero_client_guid() {
        // The field that would otherwise let a scanned host recognise one
        // ArcScan installation across visits.
        let request = negotiate_request();
        let client_guid = &request[4 + 64 + 12..4 + 64 + 28];
        assert!(client_guid.iter().all(|b| *b == 0));
    }

    #[test]
    fn a_negotiate_response_yields_the_dialect_and_the_server_guid() {
        let raw = response(0x0311, GUID, 0x0003, 0);
        let parsed = parse_negotiate_response(&raw).unwrap();
        assert_eq!(parsed.dialect.label(), Some("SMB 3.1.1"));
        assert_eq!(
            parsed.server_guid.as_deref(),
            Some("4c4c4544-0037-5a10-8051-b4c04f435331")
        );
        assert!(parsed.signing_enabled);
        assert!(parsed.signing_required);
    }

    #[test]
    fn a_dialect_is_reported_as_a_floor_and_never_as_a_version() {
        let raw = response(0x0311, GUID, 0x0001, 0);
        let parsed = parse_negotiate_response(&raw).unwrap();
        let found = evidence(&parsed);
        let banner = found
            .iter()
            .find(|e| e.key == "smb-dialect")
            .expect("the dialect is recorded");
        // The value carries its own caveat wherever it is displayed.
        assert!(banner.value.contains("or newer"));
        assert!(banner.value.contains("Samba"));
        // And it is never an OS version claim.
        assert!(!found.iter().any(|e| e.kind == EvidenceKind::OsVersion
            || e.kind == EvidenceKind::OsProduct
            || e.kind == EvidenceKind::OsBuild));
    }

    #[test]
    fn an_all_zero_server_guid_is_not_an_identity() {
        // Otherwise every server that does not set one would reconcile into a
        // single device.
        let raw = response(0x0311, [0u8; 16], 0x0001, 0);
        let parsed = parse_negotiate_response(&raw).unwrap();
        assert_eq!(parsed.server_guid, None);
        assert!(!evidence(&parsed)
            .iter()
            .any(|e| e.key == "smb_server_guid"));
    }

    #[test]
    fn signing_required_is_recorded_and_signing_merely_enabled_is_not() {
        let required = parse_negotiate_response(&response(0x0311, GUID, 0x0003, 0)).unwrap();
        assert!(evidence(&required)
            .iter()
            .any(|e| e.value == "SMB signing required"));

        let enabled = parse_negotiate_response(&response(0x0311, GUID, 0x0001, 0)).unwrap();
        assert!(!enabled.signing_required);
        assert!(!evidence(&enabled)
            .iter()
            .any(|e| e.value == "SMB signing required"));
    }

    #[test]
    fn a_refusal_is_not_read_as_a_negotiation() {
        let raw = response(0x0311, GUID, 0x0001, 0xC000_0022);
        assert!(parse_negotiate_response(&raw).is_none());
    }

    #[test]
    fn a_truncated_or_hostile_reply_is_refused_without_panicking() {
        assert!(parse_negotiate_response(&[]).is_none());
        assert!(parse_negotiate_response(&[0u8; 10]).is_none());
        assert!(parse_negotiate_response(&[0xFFu8; 200]).is_none());
        let full = response(0x0311, GUID, 0x0001, 0);
        for cut in 0..full.len() {
            // Every prefix must be refused rather than read out of bounds.
            let _ = parse_negotiate_response(&full[..cut]);
        }
    }

    #[test]
    fn a_reply_that_is_not_smb_is_refused() {
        let mut raw = response(0x0311, GUID, 0x0001, 0);
        raw[4] = 0xFF;
        assert!(parse_negotiate_response(&raw).is_none());
    }

    #[test]
    fn every_known_dialect_has_a_label_and_a_floor() {
        for code in [0x0202u16, 0x0210, 0x0300, 0x0302, 0x0311] {
            assert!(Dialect(code).label().is_some());
            assert!(Dialect(code).floor().is_some());
        }
        assert_eq!(Dialect(0x9999).label(), None);
    }
}
