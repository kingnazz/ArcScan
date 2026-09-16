//! Minimal BER for SNMPv2c GET / GETNEXT / GETBULK.
//!
//! Only the types an SNMP agent actually returns for the MIBs we walk. Hostile
//! or truncated packets fail closed.

use std::fmt;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BerError(pub &'static str);

impl fmt::Display for BerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for BerError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Oid(pub Vec<u32>);

impl Oid {
    pub fn from_slice(arcs: &[u32]) -> Self {
        Self(arcs.to_vec())
    }

    pub fn parse(s: &str) -> Result<Self, BerError> {
        let mut arcs = Vec::new();
        for part in s.split('.') {
            if part.is_empty() {
                continue;
            }
            let n: u32 = part.parse().map_err(|_| BerError("malformed OID"))?;
            arcs.push(n);
        }
        if arcs.len() < 2 {
            return Err(BerError("OID too short"));
        }
        Ok(Self(arcs))
    }

    pub fn starts_with(&self, prefix: &[u32]) -> bool {
        self.0.len() >= prefix.len() && self.0[..prefix.len()] == *prefix
    }

    pub fn suffix_after<'a>(&'a self, prefix: &[u32]) -> Option<&'a [u32]> {
        if self.starts_with(prefix) {
            Some(&self.0[prefix.len()..])
        } else {
            None
        }
    }

    pub fn to_dotted(&self) -> String {
        self.0
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(".")
    }

    pub fn append(&self, extra: &[u32]) -> Self {
        let mut v = self.0.clone();
        v.extend_from_slice(extra);
        Self(v)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_dotted())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnmpValue {
    Integer(i64),
    OctetString(Vec<u8>),
    Null,
    Oid(Oid),
    IpAddress(Ipv4Addr),
    Counter32(u32),
    Gauge32(u32),
    TimeTicks(u32),
    Counter64(u64),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
    Opaque(Vec<u8>),
}

impl SnmpValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(v) => Some(*v),
            Self::Counter32(v) | Self::Gauge32(v) | Self::TimeTicks(v) => Some(*v as i64),
            Self::Counter64(v) if *v <= i64::MAX as u64 => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Integer(v) if *v >= 0 => Some(*v as u64),
            Self::Counter32(v) | Self::Gauge32(v) | Self::TimeTicks(v) => Some(*v as u64),
            Self::Counter64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::OctetString(v) | Self::Opaque(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_utf8(&self) -> Option<String> {
        match self {
            Self::OctetString(v) => {
                let s = String::from_utf8_lossy(v).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            }
            Self::Oid(oid) => Some(oid.to_dotted()),
            Self::IpAddress(ip) => Some(ip.to_string()),
            Self::Integer(v) => Some(v.to_string()),
            _ => None,
        }
    }

    pub fn as_ip(&self) -> Option<Ipv4Addr> {
        match self {
            Self::IpAddress(ip) => Some(*ip),
            Self::OctetString(v) if v.len() == 4 => Some(Ipv4Addr::new(v[0], v[1], v[2], v[3])),
            Self::OctetString(v) => std::str::from_utf8(v).ok()?.parse().ok(),
            _ => None,
        }
    }

    pub fn is_end(&self) -> bool {
        matches!(
            self,
            Self::EndOfMibView | Self::NoSuchObject | Self::NoSuchInstance | Self::Null
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarBind {
    pub oid: Oid,
    pub value: SnmpValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnmpV2Message {
    pub community: Vec<u8>,
    pub pdu: Pdu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdu {
    pub tag: u8,
    pub request_id: i32,
    pub error_status: i32,
    pub error_index: i32,
    pub binds: Vec<VarBind>,
}

pub const PDU_GET: u8 = 0xA0;
pub const PDU_GET_NEXT: u8 = 0xA1;
pub const PDU_RESPONSE: u8 = 0xA2;
pub const PDU_GET_BULK: u8 = 0xA5;

const MAX_BER: usize = 65_536;

pub fn encode_get(community: &[u8], request_id: i32, oids: &[Oid]) -> Vec<u8> {
    let binds: Vec<VarBind> = oids
        .iter()
        .map(|oid| VarBind {
            oid: oid.clone(),
            value: SnmpValue::Null,
        })
        .collect();
    encode_message(
        community,
        Pdu {
            tag: PDU_GET,
            request_id,
            error_status: 0,
            error_index: 0,
            binds,
        },
    )
}

pub fn encode_get_bulk(
    community: &[u8],
    request_id: i32,
    non_repeaters: i32,
    max_repetitions: i32,
    oids: &[Oid],
) -> Vec<u8> {
    let binds: Vec<VarBind> = oids
        .iter()
        .map(|oid| VarBind {
            oid: oid.clone(),
            value: SnmpValue::Null,
        })
        .collect();
    encode_message(
        community,
        Pdu {
            tag: PDU_GET_BULK,
            request_id,
            error_status: non_repeaters,
            error_index: max_repetitions,
            binds,
        },
    )
}

pub fn encode_message(community: &[u8], pdu: Pdu) -> Vec<u8> {
    let mut pdu_body = Vec::new();
    encode_integer(&mut pdu_body, pdu.request_id as i64);
    encode_integer(&mut pdu_body, pdu.error_status as i64);
    encode_integer(&mut pdu_body, pdu.error_index as i64);
    let mut binds_body = Vec::new();
    for bind in &pdu.binds {
        let mut vb = Vec::new();
        encode_oid(&mut vb, &bind.oid);
        encode_value(&mut vb, &bind.value);
        encode_tlv(&mut binds_body, 0x30, &vb);
    }
    encode_tlv(&mut pdu_body, 0x30, &binds_body);

    let mut msg = Vec::new();
    encode_integer(&mut msg, 1); // SNMPv2c
    encode_octet_string(&mut msg, community);
    encode_tlv(&mut msg, pdu.tag, &pdu_body);

    let mut out = Vec::new();
    encode_tlv(&mut out, 0x30, &msg);
    out
}

pub fn decode_message(bytes: &[u8]) -> Result<SnmpV2Message, BerError> {
    if bytes.len() > MAX_BER {
        return Err(BerError("SNMP message too large"));
    }
    let (tag, body, rest) = read_tlv(bytes)?;
    if tag != 0x30 || !rest.is_empty() {
        return Err(BerError("SNMP message is not a single SEQUENCE"));
    }
    let (ver_tag, ver_body, rest) = read_tlv(body)?;
    if ver_tag != 0x02 {
        return Err(BerError("SNMP version is missing"));
    }
    let version = decode_integer(ver_body)?;
    if version != 1 {
        return Err(BerError("not an SNMPv2c message"));
    }
    let (com_tag, com_body, rest) = read_tlv(rest)?;
    if com_tag != 0x04 {
        return Err(BerError("community is missing"));
    }
    let (pdu_tag, pdu_body, rest) = read_tlv(rest)?;
    if !rest.is_empty() {
        return Err(BerError("trailing data after PDU"));
    }
    if pdu_tag != PDU_RESPONSE
        && pdu_tag != PDU_GET
        && pdu_tag != PDU_GET_NEXT
        && pdu_tag != PDU_GET_BULK
    {
        return Err(BerError("unexpected SNMP PDU"));
    }
    let pdu = decode_pdu(pdu_tag, pdu_body)?;
    Ok(SnmpV2Message {
        community: com_body.to_vec(),
        pdu,
    })
}

fn decode_pdu(tag: u8, body: &[u8]) -> Result<Pdu, BerError> {
    let (t1, b1, rest) = read_tlv(body)?;
    if t1 != 0x02 {
        return Err(BerError("request-id missing"));
    }
    let request_id = decode_integer(b1)? as i32;
    let (t2, b2, rest) = read_tlv(rest)?;
    if t2 != 0x02 {
        return Err(BerError("error-status missing"));
    }
    let error_status = decode_integer(b2)? as i32;
    let (t3, b3, rest) = read_tlv(rest)?;
    if t3 != 0x02 {
        return Err(BerError("error-index missing"));
    }
    let error_index = decode_integer(b3)? as i32;
    let (t4, b4, rest) = read_tlv(rest)?;
    if t4 != 0x30 || !rest.is_empty() {
        return Err(BerError("variable-bindings missing"));
    }
    let mut binds = Vec::new();
    let mut cur = b4;
    while !cur.is_empty() {
        let (tag, body, next) = read_tlv(cur)?;
        if tag != 0x30 {
            return Err(BerError("variable-binding is not a SEQUENCE"));
        }
        binds.push(decode_varbind(body)?);
        cur = next;
        if binds.len() > 4_096 {
            return Err(BerError("too many variable-bindings"));
        }
    }
    Ok(Pdu {
        tag,
        request_id,
        error_status,
        error_index,
        binds,
    })
}

fn decode_varbind(body: &[u8]) -> Result<VarBind, BerError> {
    let (t1, b1, rest) = read_tlv(body)?;
    if t1 != 0x06 {
        return Err(BerError("variable-binding name is not an OID"));
    }
    let oid = decode_oid(b1)?;
    let value = decode_value(rest)?;
    Ok(VarBind { oid, value })
}

fn decode_value(bytes: &[u8]) -> Result<SnmpValue, BerError> {
    let (tag, body, rest) = read_tlv(bytes)?;
    if !rest.is_empty() {
        return Err(BerError("trailing data in value"));
    }
    match tag {
        0x02 => Ok(SnmpValue::Integer(decode_integer(body)?)),
        0x04 => Ok(SnmpValue::OctetString(body.to_vec())),
        0x05 => Ok(SnmpValue::Null),
        0x06 => Ok(SnmpValue::Oid(decode_oid(body)?)),
        0x40 if body.len() == 4 => Ok(SnmpValue::IpAddress(Ipv4Addr::new(
            body[0], body[1], body[2], body[3],
        ))),
        0x41 => Ok(SnmpValue::Counter32(decode_unsigned(body)? as u32)),
        0x42 => Ok(SnmpValue::Gauge32(decode_unsigned(body)? as u32)),
        0x43 => Ok(SnmpValue::TimeTicks(decode_unsigned(body)? as u32)),
        0x44 => Ok(SnmpValue::Opaque(body.to_vec())),
        0x46 => Ok(SnmpValue::Counter64(decode_unsigned(body)?)),
        0x80 => Ok(SnmpValue::NoSuchObject),
        0x81 => Ok(SnmpValue::NoSuchInstance),
        0x82 => Ok(SnmpValue::EndOfMibView),
        _ => Err(BerError("unsupported SNMP value type")),
    }
}

fn encode_value(out: &mut Vec<u8>, value: &SnmpValue) {
    match value {
        SnmpValue::Integer(v) => encode_integer(out, *v),
        SnmpValue::OctetString(v) => encode_octet_string(out, v),
        SnmpValue::Null => encode_tlv(out, 0x05, &[]),
        SnmpValue::Oid(oid) => encode_oid(out, oid),
        SnmpValue::IpAddress(ip) => encode_tlv(out, 0x40, &ip.octets()),
        SnmpValue::Counter32(v) => encode_unsigned(out, 0x41, *v as u64),
        SnmpValue::Gauge32(v) => encode_unsigned(out, 0x42, *v as u64),
        SnmpValue::TimeTicks(v) => encode_unsigned(out, 0x43, *v as u64),
        SnmpValue::Counter64(v) => encode_unsigned(out, 0x46, *v),
        SnmpValue::NoSuchObject => encode_tlv(out, 0x80, &[]),
        SnmpValue::NoSuchInstance => encode_tlv(out, 0x81, &[]),
        SnmpValue::EndOfMibView => encode_tlv(out, 0x82, &[]),
        SnmpValue::Opaque(v) => encode_tlv(out, 0x44, v),
    }
}

fn encode_tlv(out: &mut Vec<u8>, tag: u8, body: &[u8]) {
    out.push(tag);
    encode_length(out, body.len());
    out.extend_from_slice(body);
}

fn encode_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xFF {
        out.push(0x81);
        out.push(len as u8);
    } else if len <= 0xFFFF {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push(len as u8);
    } else {
        out.push(0x83);
        out.push((len >> 16) as u8);
        out.push((len >> 8) as u8);
        out.push(len as u8);
    }
}

fn encode_integer(out: &mut Vec<u8>, value: i64) {
    let mut bytes = value.to_be_bytes().to_vec();
    while bytes.len() > 1
        && ((bytes[0] == 0x00 && bytes[1] & 0x80 == 0)
            || (bytes[0] == 0xFF && bytes[1] & 0x80 != 0))
    {
        bytes.remove(0);
    }
    encode_tlv(out, 0x02, &bytes);
}

fn encode_unsigned(out: &mut Vec<u8>, tag: u8, value: u64) {
    let mut bytes = value.to_be_bytes().to_vec();
    while bytes.len() > 1 && bytes[0] == 0x00 && bytes[1] & 0x80 == 0 {
        bytes.remove(0);
    }
    if bytes[0] & 0x80 != 0 {
        bytes.insert(0, 0x00);
    }
    encode_tlv(out, tag, &bytes);
}

fn encode_octet_string(out: &mut Vec<u8>, value: &[u8]) {
    encode_tlv(out, 0x04, value);
}

fn encode_oid(out: &mut Vec<u8>, oid: &Oid) {
    if oid.0.len() < 2 {
        encode_tlv(out, 0x06, &[]);
        return;
    }
    let mut body = Vec::new();
    body.push((40 * oid.0[0].min(2) + oid.0[1].min(39)) as u8);
    for &arc in &oid.0[2..] {
        encode_base128(&mut body, arc);
    }
    encode_tlv(out, 0x06, &body);
}

fn encode_base128(out: &mut Vec<u8>, mut value: u32) {
    if value == 0 {
        out.push(0);
        return;
    }
    let mut tmp = [0u8; 5];
    let mut n = 0;
    while value > 0 {
        tmp[n] = (value & 0x7F) as u8;
        value >>= 7;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        let mut b = tmp[n];
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
    }
}

fn read_tlv(bytes: &[u8]) -> Result<(u8, &[u8], &[u8]), BerError> {
    if bytes.is_empty() {
        return Err(BerError("truncated TLV"));
    }
    let tag = bytes[0];
    let (len, hdr) = decode_length(&bytes[1..])?;
    let start = 1 + hdr;
    if bytes.len() < start + len {
        return Err(BerError("truncated TLV body"));
    }
    Ok((tag, &bytes[start..start + len], &bytes[start + len..]))
}

fn decode_length(bytes: &[u8]) -> Result<(usize, usize), BerError> {
    if bytes.is_empty() {
        return Err(BerError("truncated length"));
    }
    let first = bytes[0];
    if first < 0x80 {
        return Ok((first as usize, 1));
    }
    let count = (first & 0x7F) as usize;
    if count == 0 || count > 3 || bytes.len() < 1 + count {
        return Err(BerError("invalid BER length"));
    }
    let mut len = 0usize;
    for b in &bytes[1..1 + count] {
        len = (len << 8) | (*b as usize);
    }
    if len > MAX_BER {
        return Err(BerError("BER length too large"));
    }
    Ok((len, 1 + count))
}

fn decode_integer(bytes: &[u8]) -> Result<i64, BerError> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(BerError("invalid INTEGER"));
    }
    let mut value: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    for b in bytes {
        value = (value << 8) | (*b as i64);
    }
    Ok(value)
}

fn decode_unsigned(bytes: &[u8]) -> Result<u64, BerError> {
    if bytes.is_empty() || bytes.len() > 9 {
        return Err(BerError("invalid unsigned"));
    }
    let start = if bytes.len() > 1 && bytes[0] == 0 {
        1
    } else {
        0
    };
    let mut value = 0u64;
    for b in &bytes[start..] {
        value = (value << 8) | (*b as u64);
    }
    Ok(value)
}

fn decode_oid(bytes: &[u8]) -> Result<Oid, BerError> {
    if bytes.is_empty() {
        return Err(BerError("empty OID"));
    }
    let first = bytes[0];
    let mut arcs = vec![(first / 40) as u32, (first % 40) as u32];
    let mut i = 1;
    while i < bytes.len() {
        let mut value: u32 = 0;
        loop {
            if i >= bytes.len() {
                return Err(BerError("truncated OID"));
            }
            let b = bytes[i];
            i += 1;
            value = value
                .checked_shl(7)
                .and_then(|v| v.checked_add((b & 0x7F) as u32))
                .ok_or(BerError("OID arc overflow"))?;
            if b & 0x80 == 0 {
                break;
            }
        }
        arcs.push(value);
        if arcs.len() > 128 {
            return Err(BerError("OID too long"));
        }
    }
    Ok(Oid(arcs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_round_trip() {
        let oid = Oid::parse("1.3.6.1.2.1.1.5.0").unwrap();
        let mut buf = Vec::new();
        encode_oid(&mut buf, &oid);
        let (tag, body, rest) = read_tlv(&buf).unwrap();
        assert_eq!(tag, 0x06);
        assert!(rest.is_empty());
        assert_eq!(decode_oid(body).unwrap(), oid);
    }

    #[test]
    fn get_message_round_trips() {
        let oid = Oid::parse("1.3.6.1.2.1.1.5.0").unwrap();
        let bytes = encode_get(b"site-read", 7, std::slice::from_ref(&oid));
        let msg = decode_message(&bytes).unwrap();
        assert_eq!(msg.community, b"site-read");
        assert_eq!(msg.pdu.request_id, 7);
        assert_eq!(msg.pdu.binds[0].oid, oid);
        assert_eq!(msg.pdu.binds[0].value, SnmpValue::Null);
    }

    #[test]
    fn integer_negative_and_large() {
        for v in [
            0i64,
            1,
            127,
            128,
            -1,
            -128,
            -129,
            i32::MIN as i64,
            2_147_483_647,
        ] {
            let mut buf = Vec::new();
            encode_integer(&mut buf, v);
            let (tag, body, _) = read_tlv(&buf).unwrap();
            assert_eq!(tag, 0x02);
            assert_eq!(decode_integer(body).unwrap(), v);
        }
    }

    #[test]
    fn truncated_packet_is_refused() {
        assert!(decode_message(&[0x30, 0x20, 0x02]).is_err());
    }
}
