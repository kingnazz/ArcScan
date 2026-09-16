//! What a device's TLS certificate says about it.
//!
//! An embedded appliance almost always presents a self-signed certificate its
//! own firmware generated, and the subject of that certificate is written by
//! the manufacturer: `CN=DiskStation`, `CN=iDRAC-7SZ1B43`,
//! `O=Ubiquiti Inc.`. On a locked-down box with every other port closed, it is
//! frequently the only self-description available — and on a Dell management
//! controller the common name contains the service tag, which is a genuine
//! hardware identity.
//!
//! # Scope
//!
//! This reads metadata. It does not validate the certificate, check its dates,
//! verify a chain, or make any trust decision — there is no trust decision here
//! to make, because nothing confidential is sent. The handshake stops as soon
//! as the certificate has been read.
//!
//! # Why the hello offers TLS 1.2
//!
//! TLS 1.3 encrypts the certificate message, so a 1.3 handshake reveals
//! nothing. The probe therefore offers 1.2 as its maximum, which every server
//! that supports 1.3 also supports. This weakens nothing: it is a read-only
//! probe that sends no data, and the server's own configuration for real
//! clients is untouched.

use crate::discovery::model::{
    sanitize_field, Confidence, DiscoverySource, Evidence, EvidenceKind,
};

use super::der;
use super::fingerprint::match_signature;

/// Most handshake bytes this will read.
pub const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;

/// What a presented certificate said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsFingerprint {
    /// The subject's common name.
    pub common_name: Option<String>,
    /// The subject's organisation.
    pub organization: Option<String>,
    /// The issuer's common name. Equal to `common_name` on a self-signed
    /// appliance certificate, which is itself informative.
    pub issuer_common_name: Option<String>,
    /// DNS names from the subject-alternative-name extension.
    pub dns_names: Vec<String>,
}

impl TlsFingerprint {
    pub fn is_empty(&self) -> bool {
        self.common_name.is_none()
            && self.organization.is_none()
            && self.issuer_common_name.is_none()
            && self.dns_names.is_empty()
    }

    /// True when the subject and issuer match, i.e. the appliance signed its
    /// own certificate. Ordinary for embedded hardware, and a hint that the
    /// subject was written by the firmware rather than by a certificate
    /// authority.
    pub fn self_signed(&self) -> bool {
        match (&self.common_name, &self.issuer_common_name) {
            (Some(subject), Some(issuer)) => subject == issuer,
            _ => false,
        }
    }
}

/// A minimal TLS 1.2 ClientHello.
///
/// No SNI: the probe connects to an address, not a name, and inventing a name
/// to put in the extension would be telling the device something untrue.
pub fn client_hello() -> Vec<u8> {
    // A fixed, non-random client random. It is not used for any key exchange
    // that completes, and a fixed value cannot leak entropy or act as a cookie
    // that identifies this installation across scans.
    let client_random = [0x41u8; 32];

    let cipher_suites: [u16; 10] = [
        0xC02F, 0xC030, 0xC02B, 0xC02C, 0xC013, 0xC014, 0x009C, 0x009D, 0x002F, 0x0035,
    ];

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x03]); // client_version: TLS 1.2
    body.extend_from_slice(&client_random);
    body.push(0x00); // session id length
    body.extend_from_slice(&((cipher_suites.len() * 2) as u16).to_be_bytes());
    for suite in cipher_suites {
        body.extend_from_slice(&suite.to_be_bytes());
    }
    body.extend_from_slice(&[0x01, 0x00]); // compression: null only

    // Extensions: supported groups and point formats, which some servers
    // require before they will select an ECDHE suite and get as far as sending
    // a certificate.
    let mut extensions = Vec::new();
    // supported_groups: secp256r1, secp384r1, x25519
    extensions.extend_from_slice(&[0x00, 0x0A, 0x00, 0x08, 0x00, 0x06]);
    extensions.extend_from_slice(&[0x00, 0x17, 0x00, 0x18, 0x00, 0x1D]);
    // ec_point_formats: uncompressed
    extensions.extend_from_slice(&[0x00, 0x0B, 0x00, 0x02, 0x01, 0x00]);
    // signature_algorithms, required by TLS 1.2 servers that check.
    extensions.extend_from_slice(&[0x00, 0x0D, 0x00, 0x0C, 0x00, 0x0A]);
    extensions.extend_from_slice(&[0x04, 0x01, 0x04, 0x03, 0x05, 0x01, 0x02, 0x01, 0x02, 0x03]);
    body.extend_from_slice(&(extensions.len() as u16).to_be_bytes());
    body.extend_from_slice(&extensions);

    // Handshake header: type 0x01 (client_hello) and a 24-bit length.
    let mut handshake = vec![0x01];
    let length = body.len();
    handshake.extend_from_slice(&[
        ((length >> 16) & 0xFF) as u8,
        ((length >> 8) & 0xFF) as u8,
        (length & 0xFF) as u8,
    ]);
    handshake.extend_from_slice(&body);

    // Record header: handshake, TLS 1.0 for maximum compatibility of the
    // record layer itself, and a 16-bit length.
    let mut record = vec![0x16, 0x03, 0x01];
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

/// Pull the first certificate's DER out of a stream of TLS records.
///
/// Walks the record layer, reassembles handshake messages, and returns the
/// first certificate in the first `Certificate` message. Returns `None` for an
/// alert, a TLS 1.3 handshake (whose certificate is encrypted) or anything
/// malformed.
pub fn first_certificate(stream: &[u8]) -> Option<Vec<u8>> {
    let mut handshake = Vec::new();
    let mut pos = 0usize;
    while pos + 5 <= stream.len() && handshake.len() < MAX_HANDSHAKE_BYTES {
        let content_type = stream[pos];
        let length = u16::from_be_bytes([stream[pos + 3], stream[pos + 4]]) as usize;
        let start = pos + 5;
        let end = start.checked_add(length)?;
        if end > stream.len() {
            break;
        }
        // 0x16 is handshake. 0x15 is an alert, which means the server declined
        // and there is nothing further to read.
        if content_type == 0x15 {
            break;
        }
        if content_type == 0x16 {
            handshake.extend_from_slice(&stream[start..end]);
        }
        pos = end;
    }

    // Walk the reassembled handshake messages for type 11, Certificate.
    let mut pos = 0usize;
    while pos + 4 <= handshake.len() {
        let msg_type = handshake[pos];
        let length = ((handshake[pos + 1] as usize) << 16)
            | ((handshake[pos + 2] as usize) << 8)
            | handshake[pos + 3] as usize;
        let start = pos + 4;
        let end = start.checked_add(length)?;
        if end > handshake.len() {
            break;
        }
        if msg_type == 11 {
            let body = &handshake[start..end];
            // Certificate message: a 24-bit list length, then each certificate
            // as a 24-bit length followed by its DER.
            if body.len() < 6 {
                return None;
            }
            let cert_len =
                ((body[3] as usize) << 16) | ((body[4] as usize) << 8) | body[5] as usize;
            let cert_start = 6usize;
            let cert_end = cert_start.checked_add(cert_len)?;
            if cert_end > body.len() || cert_len == 0 {
                return None;
            }
            return Some(body[cert_start..cert_end].to_vec());
        }
        pos = end;
    }
    None
}

// OIDs, as their DER content bytes (the value of the OID TLV).
const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const OID_ORGANIZATION: &[u8] = &[0x55, 0x04, 0x0A];
const OID_SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1D, 0x11];

/// Read the subject, issuer and SAN out of a certificate's DER.
pub fn parse_certificate(cert: &[u8]) -> Option<TlsFingerprint> {
    let certificate = der::read(cert, 0)?;
    if certificate.tag != der::TAG_SEQUENCE {
        return None;
    }
    let tbs = der::children(certificate.value).into_iter().next()?;
    if tbs.tag != der::TAG_SEQUENCE {
        return None;
    }
    let fields = der::children(tbs.value);

    // The optional [0] EXPLICIT version comes first when present. Skipping it
    // is what keeps the positional reads below aligned on both v1 and v3
    // certificates.
    let offset = usize::from(fields.first().is_some_and(|f| f.tag == 0xA0));
    let issuer = fields.get(offset + 2).filter(|f| f.tag == der::TAG_SEQUENCE);
    let subject = fields.get(offset + 4).filter(|f| f.tag == der::TAG_SEQUENCE);

    let mut out = TlsFingerprint::default();
    if let Some(subject) = subject {
        out.common_name = rdn_value(subject.value, OID_COMMON_NAME);
        out.organization = rdn_value(subject.value, OID_ORGANIZATION);
    }
    if let Some(issuer) = issuer {
        out.issuer_common_name = rdn_value(issuer.value, OID_COMMON_NAME);
    }

    // Extensions live in the [3] EXPLICIT wrapper, when there is one.
    if let Some(extensions) = fields.iter().find(|f| f.tag == 0xA3) {
        if let Some(seq) = der::children(extensions.value)
            .into_iter()
            .find(|c| c.tag == der::TAG_SEQUENCE)
        {
            out.dns_names = subject_alt_names(seq.value);
        }
    }

    (!out.is_empty()).then_some(out)
}

/// Find one attribute in an RDNSequence.
fn rdn_value(rdn_sequence: &[u8], oid: &[u8]) -> Option<String> {
    for rdn in der::children(rdn_sequence) {
        if rdn.tag != der::TAG_SET {
            continue;
        }
        for attribute in der::children(rdn.value) {
            if attribute.tag != der::TAG_SEQUENCE {
                continue;
            }
            let parts = der::children(attribute.value);
            let Some(kind) = parts.first() else { continue };
            if kind.tag != der::TAG_OID || kind.value != oid {
                continue;
            }
            let Some(value) = parts.get(1) else { continue };
            if let Some(text) = der::decode_string(value.tag, value.value) {
                if let Some(clean) = sanitize_field(&text) {
                    return Some(clean);
                }
            }
        }
    }
    None
}

/// DNS names from the subject-alternative-name extension.
fn subject_alt_names(extensions: &[u8]) -> Vec<String> {
    const MAX_NAMES: usize = 16;
    for extension in der::children(extensions) {
        if extension.tag != der::TAG_SEQUENCE {
            continue;
        }
        let parts = der::children(extension.value);
        let Some(oid) = parts.first() else { continue };
        if oid.tag != der::TAG_OID || oid.value != OID_SUBJECT_ALT_NAME {
            continue;
        }
        // The value is an OCTET STRING, optionally preceded by a critical flag.
        let Some(octets) = parts
            .iter()
            .find(|p| p.tag == der::TAG_OCTET_STRING && p.tag != der::TAG_BOOLEAN)
        else {
            continue;
        };
        let Some(names) = der::read(octets.value, 0) else {
            continue;
        };
        return der::children(names.value)
            .into_iter()
            // [2] IMPLICIT dNSName.
            .filter(|name| name.tag == 0x82)
            .filter_map(|name| sanitize_field(&String::from_utf8_lossy(name.value)))
            .take(MAX_NAMES)
            .collect();
    }
    Vec::new()
}

/// Turn a certificate fingerprint into discovery evidence.
pub fn evidence(fingerprint: &TlsFingerprint, port: u16) -> Vec<Evidence> {
    let mut out = Vec::new();
    let key = port.to_string();

    if let Some(cn) = &fingerprint.common_name {
        out.push(Evidence::new(
            DiscoverySource::Tls,
            EvidenceKind::CertificateSubject,
            &format!("cn:{key}"),
            cn,
            Confidence::Medium,
        ));
    }
    if let Some(org) = &fingerprint.organization {
        out.push(Evidence::new(
            DiscoverySource::Tls,
            EvidenceKind::CertificateSubject,
            &format!("o:{key}"),
            org,
            Confidence::Medium,
        ));
    }

    // Identity from whichever subject field a signature recognises.
    for candidate in [
        fingerprint.common_name.as_deref(),
        fingerprint.organization.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(signature) = match_signature(candidate) else {
            continue;
        };
        if let Some(manufacturer) = signature.manufacturer {
            out.push(Evidence::new(
                DiscoverySource::Tls,
                EvidenceKind::Manufacturer,
                "",
                manufacturer,
                signature.confidence,
            ));
        }
        if let Some(model) = signature.model {
            out.push(Evidence::new(
                DiscoverySource::Tls,
                EvidenceKind::Model,
                "",
                model,
                signature.confidence,
            ));
        }
        break;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a DER TLV.
    fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if value.len() < 128 {
            out.push(value.len() as u8);
        } else {
            out.push(0x82);
            out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(value);
        out
    }

    /// An RDNSequence carrying one attribute.
    fn rdn(oid: &[u8], text: &str) -> Vec<u8> {
        let attribute = tlv(
            0x30,
            &[tlv(0x06, oid), tlv(0x13, text.as_bytes())].concat(),
        );
        tlv(0x31, &attribute)
    }

    /// A certificate with the given subject and issuer common names.
    fn certificate(subject_cn: &str, org: Option<&str>, issuer_cn: &str) -> Vec<u8> {
        let mut subject = rdn(OID_COMMON_NAME, subject_cn);
        if let Some(org) = org {
            subject.extend(rdn(OID_ORGANIZATION, org));
        }
        let tbs = [
            tlv(0xA0, &tlv(0x02, &[0x02])),     // version v3
            tlv(0x02, &[0x01, 0x23]),           // serial
            tlv(0x30, &tlv(0x06, &[0x2A])),     // signature algorithm
            tlv(0x30, &rdn(OID_COMMON_NAME, issuer_cn)), // issuer
            tlv(0x30, &[]),                     // validity
            tlv(0x30, &subject),                // subject
            tlv(0x30, &[]),                     // subjectPublicKeyInfo
        ]
        .concat();
        tlv(0x30, &[tlv(0x30, &tbs), tlv(0x30, &[]), tlv(0x03, &[0x00])].concat())
    }

    #[test]
    fn a_self_signed_appliance_certificate_yields_its_common_name() {
        let cert = certificate("DiskStation", Some("Synology Inc."), "DiskStation");
        let parsed = parse_certificate(&cert).unwrap();
        assert_eq!(parsed.common_name.as_deref(), Some("DiskStation"));
        assert_eq!(parsed.organization.as_deref(), Some("Synology Inc."));
        assert!(parsed.self_signed());
    }

    #[test]
    fn a_certificate_signed_by_someone_else_is_not_self_signed() {
        let cert = certificate("files.corp.example", None, "Corp Issuing CA");
        let parsed = parse_certificate(&cert).unwrap();
        assert!(!parsed.self_signed());
        assert_eq!(parsed.issuer_common_name.as_deref(), Some("Corp Issuing CA"));
    }

    #[test]
    fn an_idrac_certificate_identifies_a_management_controller() {
        // The common name on a Dell BMC carries the service tag, and this is
        // often the only string a locked-down controller will offer.
        let cert = certificate("iDRAC-7SZ1B43", Some("Dell Inc."), "iDRAC-7SZ1B43");
        let parsed = parse_certificate(&cert).unwrap();
        let found = evidence(&parsed, 443);
        assert_eq!(
            found
                .iter()
                .find(|e| e.kind == EvidenceKind::Model)
                .map(|e| e.value.as_str()),
            Some("iDRAC")
        );
        // And the raw subject is kept for a technician to read.
        assert!(found
            .iter()
            .any(|e| e.kind == EvidenceKind::CertificateSubject && e.value == "iDRAC-7SZ1B43"));
    }

    #[test]
    fn a_certificate_with_nothing_readable_yields_nothing() {
        assert!(parse_certificate(&[]).is_none());
        assert!(parse_certificate(&[0x30, 0x00]).is_none());
        assert!(parse_certificate(b"not a certificate at all").is_none());
    }

    #[test]
    fn hostile_certificate_bytes_do_not_panic() {
        for pattern in [
            vec![0xFFu8; 1024],
            vec![0x30u8; 1024],
            vec![0x00u8; 1024],
            (0u8..=255).cycle().take(2048).collect::<Vec<u8>>(),
        ] {
            let _ = parse_certificate(&pattern);
        }
        // And every prefix of a well-formed certificate.
        let cert = certificate("DiskStation", None, "DiskStation");
        for cut in 0..cert.len() {
            let _ = parse_certificate(&cert[..cut]);
        }
    }

    #[test]
    fn the_client_hello_is_a_well_formed_tls_record() {
        let hello = client_hello();
        assert_eq!(hello[0], 0x16); // handshake
        let record_len = u16::from_be_bytes([hello[3], hello[4]]) as usize;
        assert_eq!(record_len, hello.len() - 5);
        assert_eq!(hello[5], 0x01); // client_hello
        let handshake_len =
            ((hello[6] as usize) << 16) | ((hello[7] as usize) << 8) | hello[8] as usize;
        assert_eq!(handshake_len, hello.len() - 9);
    }

    #[test]
    fn the_client_hello_offers_tls_1_2_so_the_certificate_stays_readable() {
        let hello = client_hello();
        assert_eq!(&hello[9..11], &[0x03, 0x03]);
    }

    #[test]
    fn a_certificate_message_is_found_in_a_record_stream() {
        let cert = certificate("DiskStation", None, "DiskStation");
        // Certificate handshake message: list length, then one entry.
        let mut body = Vec::new();
        let list_len = cert.len() + 3;
        body.extend_from_slice(&[
            ((list_len >> 16) & 0xFF) as u8,
            ((list_len >> 8) & 0xFF) as u8,
            (list_len & 0xFF) as u8,
        ]);
        body.extend_from_slice(&[
            ((cert.len() >> 16) & 0xFF) as u8,
            ((cert.len() >> 8) & 0xFF) as u8,
            (cert.len() & 0xFF) as u8,
        ]);
        body.extend_from_slice(&cert);

        let mut handshake = vec![11u8];
        handshake.extend_from_slice(&[
            ((body.len() >> 16) & 0xFF) as u8,
            ((body.len() >> 8) & 0xFF) as u8,
            (body.len() & 0xFF) as u8,
        ]);
        handshake.extend_from_slice(&body);

        let mut record = vec![0x16, 0x03, 0x03];
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);

        let found = first_certificate(&record).unwrap();
        assert_eq!(found, cert);
    }

    #[test]
    fn an_alert_instead_of_a_handshake_yields_no_certificate() {
        // 0x15 is an alert: the server declined, and there is nothing to read.
        let record = vec![0x15, 0x03, 0x03, 0x00, 0x02, 0x02, 0x28];
        assert!(first_certificate(&record).is_none());
    }

    #[test]
    fn a_truncated_record_stream_yields_no_certificate_rather_than_panicking() {
        assert!(first_certificate(&[]).is_none());
        assert!(first_certificate(&[0x16, 0x03, 0x03, 0xFF, 0xFF]).is_none());
        for pattern in [vec![0xFFu8; 256], vec![0x16u8; 256]] {
            let _ = first_certificate(&pattern);
        }
    }
}
