//! Multicast DNS: query construction and a bounded, panic-free parser.
//!
//! # Why a parser rather than a crate
//!
//! Every maintained mDNS crate ArcScan could use brings a *responder* — a
//! long-lived background listener that joins the group, answers queries and
//! keeps a cache. That is the opposite of what this release promises: a bounded,
//! read-only, one-shot query with a hard time budget and no residency on the
//! network. The parsing itself is a few hundred lines of well-specified
//! structure, so the honest trade is to write exactly the subset needed and test
//! it hard against malformed input, rather than take a dependency whose main
//! feature has to be switched off.
//!
//! # What the parser refuses
//!
//! Everything below is enforced before a single byte is copied into a `String`:
//!
//! * a packet larger than [`MAX_PACKET_BYTES`]
//! * more than [`MAX_RECORDS`] records in one packet
//! * a name longer than 255 bytes, or a label longer than 63
//! * a compression pointer that does not point strictly backwards
//! * more than [`MAX_POINTER_JUMPS`] pointer jumps while reading one name
//! * a record whose declared length runs past the end of the packet
//! * more than [`MAX_TXT_PAIRS`] TXT entries, or an over-long key or value
//!
//! Nothing here can panic on hostile input: every read goes through the bounds-
//! checked [`Reader`], and malformed UTF-8 is replaced rather than rejected, so
//! one bad byte in a name does not discard an otherwise useful answer.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Largest mDNS packet accepted. RFC 6762 permits responses up to 9000 bytes
/// when the network supports them; anything past that is not a real answer.
pub const MAX_PACKET_BYTES: usize = 9_000;

/// Most records parsed out of one packet, across all four sections.
pub const MAX_RECORDS: usize = 128;

/// Most compression-pointer jumps allowed while reading one name. Combined with
/// the strictly-backwards rule this makes a pointer loop impossible; the counter
/// is belt and braces for a pathological but legal chain.
pub const MAX_POINTER_JUMPS: usize = 16;

/// Most key/value pairs kept from one TXT record.
pub const MAX_TXT_PAIRS: usize = 32;

/// Longest TXT key and value kept, in bytes.
pub const MAX_TXT_KEY: usize = 64;
pub const MAX_TXT_VALUE: usize = 192;

/// The DNS-SD meta-query that asks a network which service types exist on it.
pub const SERVICE_ENUMERATION: &str = "_services._dns-sd._udp.local";

/// Service types ArcScan asks about directly when enumeration returns nothing
/// useful, chosen because each one maps onto a device type a person recognises.
///
/// This list is a *fallback*. The normal path enumerates first and then asks
/// only about what the network said it has, which is both less traffic and more
/// complete than any fixed list can be.
pub const FALLBACK_SERVICES: &[&str] = &[
    "_device-info._tcp.local",
    "_workstation._tcp.local",
    "_ipp._tcp.local",
    "_printer._tcp.local",
    "_googlecast._tcp.local",
    "_airplay._tcp.local",
    "_smb._tcp.local",
    "_hap._tcp.local",
];

/// Record types the parser understands. Everything else is skipped by length.
pub const TYPE_A: u16 = 1;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_SRV: u16 = 33;
pub const TYPE_ANY: u16 = 255;

const CLASS_IN: u16 = 1;
/// RFC 6762 §5.4: the top bit of the class field asks for a unicast reply.
const UNICAST_RESPONSE: u16 = 0x8000;

/// One parsed resource record, reduced to the fields ArcScan uses.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordData {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Ptr(String),
    Srv {
        port: u16,
        target: String,
    },
    Txt(BTreeMap<String, String>),
    /// A record of a type we do not decode, kept only so counts and limits
    /// reflect the packet as it really was.
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub ttl: u32,
    pub data: RecordData,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Message {
    pub answers: Vec<Record>,
}

/// A bounds-checked cursor over a packet. Every read returns an `Option`, so a
/// truncated packet ends parsing instead of indexing out of range.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn u8(&mut self) -> Option<u8> {
        let v = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }

    fn u16(&mut self) -> Option<u16> {
        let hi = self.u8()? as u16;
        let lo = self.u8()? as u16;
        Some((hi << 8) | lo)
    }

    fn u32(&mut self) -> Option<u32> {
        let hi = self.u16()? as u32;
        let lo = self.u16()? as u32;
        Some((hi << 16) | lo)
    }

    fn slice(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let out = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    /// Read a DNS name, following compression pointers.
    ///
    /// Two independent rules make a loop impossible: a pointer must target an
    /// offset *strictly before* the lowest one already visited, and the number
    /// of jumps is capped. The cursor is left just past the name in the original
    /// stream, not wherever the last pointer led.
    fn name(&mut self) -> Option<String> {
        let mut labels: Vec<String> = Vec::new();
        let mut total = 0usize;
        let mut jumps = 0usize;
        let mut cursor = self.pos;
        let mut lowest_pointer = usize::MAX;
        let mut resume: Option<usize> = None;

        loop {
            let len = *self.buf.get(cursor)? as usize;
            cursor += 1;

            match len & 0xC0 {
                0x00 => {
                    if len == 0 {
                        break;
                    }
                    if len > 63 {
                        return None;
                    }
                    total += len + 1;
                    if total > 255 {
                        return None;
                    }
                    let end = cursor.checked_add(len)?;
                    let raw = self.buf.get(cursor..end)?;
                    cursor = end;
                    labels.push(String::from_utf8_lossy(raw).into_owned());
                }
                0xC0 => {
                    jumps += 1;
                    if jumps > MAX_POINTER_JUMPS {
                        return None;
                    }
                    let lo = *self.buf.get(cursor)? as usize;
                    cursor += 1;
                    if resume.is_none() {
                        resume = Some(cursor);
                    }
                    let target = ((len & 0x3F) << 8) | lo;
                    // Strictly backwards: a pointer to itself, forwards, or to
                    // anywhere at or after a pointer already followed would
                    // allow a cycle.
                    if target >= lowest_pointer || target >= self.buf.len() {
                        return None;
                    }
                    lowest_pointer = target;
                    cursor = target;
                }
                // 0x40 and 0x80 are reserved label types. A packet using one is
                // not something ArcScan should guess at.
                _ => return None,
            }
        }

        self.pos = resume.unwrap_or(cursor);
        Some(labels.join("."))
    }
}

/// Build a query packet asking about every name in `names`.
///
/// The unicast-response bit is set on each question, which is what RFC 6762
/// §5.1 asks of a *one-shot* querier: ArcScan sends from an ephemeral port,
/// collects direct replies for a few seconds and closes the socket. It never
/// binds port 5353, so it cannot collide with the Bonjour or Avahi responder
/// already running on the machine, and it never becomes a second responder on
/// the network.
pub fn build_query(names: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + names.len() * 32);
    // ID 0: mDNS responses are matched on content, not on a transaction id.
    out.extend_from_slice(&[0x00, 0x00]);
    // Flags: standard query, recursion not desired.
    out.extend_from_slice(&[0x00, 0x00]);
    out.extend_from_slice(&(names.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00]); // answers
    out.extend_from_slice(&[0x00, 0x00]); // authority
    out.extend_from_slice(&[0x00, 0x00]); // additional

    for name in names {
        encode_name(name, &mut out);
        out.extend_from_slice(&TYPE_PTR.to_be_bytes());
        out.extend_from_slice(&(CLASS_IN | UNICAST_RESPONSE).to_be_bytes());
    }
    out
}

/// Build a query for the records that describe one service instance: its SRV
/// (host and port) and its TXT (properties), asked as ANY in a single question.
pub fn build_instance_query(instances: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + instances.len() * 48);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&(instances.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for instance in instances {
        encode_name(instance, &mut out);
        out.extend_from_slice(&TYPE_ANY.to_be_bytes());
        out.extend_from_slice(&(CLASS_IN | UNICAST_RESPONSE).to_be_bytes());
    }
    out
}

/// Write a dotted name in DNS label form. Labels longer than 63 bytes are
/// truncated rather than dropped, because a query ArcScan built is never
/// hostile — this only keeps a mistake from producing an invalid packet.
fn encode_name(name: &str, out: &mut Vec<u8>) {
    for label in name.split('.').filter(|l| !l.is_empty()) {
        let bytes = label.as_bytes();
        let len = bytes.len().min(63);
        out.push(len as u8);
        out.extend_from_slice(&bytes[..len]);
    }
    out.push(0);
}

/// Parse a response packet into the records ArcScan understands.
///
/// Returns `None` only for a packet that is not a DNS message at all. A packet
/// that runs out mid-record yields the records read so far: a truncated answer
/// is still an answer, and discarding it would lose real devices on a lossy
/// network.
pub fn parse(packet: &[u8]) -> Option<Message> {
    if packet.len() > MAX_PACKET_BYTES || packet.len() < 12 {
        return None;
    }
    let mut r = Reader::new(packet);
    let _id = r.u16()?;
    let flags = r.u16()?;
    // Bit 15 set means this is a response. A query echoed back to us (some
    // networks reflect multicast) carries no answers and is not worth parsing.
    if flags & 0x8000 == 0 {
        return None;
    }
    let counts = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
    let questions = counts[0] as usize;

    // Skip the question section: a response repeats what was asked, and ArcScan
    // matches on the answers.
    for _ in 0..questions.min(MAX_RECORDS) {
        r.name()?;
        r.u16()?;
        r.u16()?;
    }

    let declared: usize = counts[1..].iter().map(|c| *c as usize).sum();
    let mut answers = Vec::new();
    for _ in 0..declared.min(MAX_RECORDS) {
        match read_record(&mut r) {
            Some(record) => answers.push(record),
            None => break,
        }
    }
    Some(Message { answers })
}

fn read_record(r: &mut Reader<'_>) -> Option<Record> {
    let name = r.name()?;
    let rtype = r.u16()?;
    let _class = r.u16()?;
    let ttl = r.u32()?;
    let rdlen = r.u16()? as usize;
    let start = r.pos;
    let body = r.slice(rdlen)?;

    let data = match rtype {
        TYPE_A if body.len() == 4 => {
            RecordData::A(Ipv4Addr::new(body[0], body[1], body[2], body[3]))
        }
        TYPE_AAAA if body.len() == 16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(body);
            RecordData::Aaaa(Ipv6Addr::from(octets))
        }
        TYPE_PTR => {
            // The target may use compression, so it is read against the whole
            // packet from the record's own offset rather than from `body`.
            let mut inner = Reader {
                buf: r.buf,
                pos: start,
            };
            RecordData::Ptr(inner.name()?)
        }
        TYPE_SRV if body.len() >= 6 => {
            let port = u16::from_be_bytes([body[4], body[5]]);
            let mut inner = Reader {
                buf: r.buf,
                pos: start + 6,
            };
            let target = inner.name().unwrap_or_default();
            RecordData::Srv { port, target }
        }
        TYPE_TXT => RecordData::Txt(parse_txt(body)),
        _ => RecordData::Other,
    };

    Some(Record {
        name,
        rtype,
        ttl,
        data,
    })
}

/// Parse the length-prefixed strings of a TXT record into key/value pairs.
///
/// A malformed entry stops the record rather than the packet, and every bound is
/// applied as the map is built, so a TXT record advertising ten thousand keys
/// costs [`MAX_TXT_PAIRS`] entries and nothing more.
fn parse_txt(body: &[u8]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut pos = 0usize;
    while pos < body.len() && out.len() < MAX_TXT_PAIRS {
        let len = body[pos] as usize;
        pos += 1;
        let Some(entry) = body.get(pos..pos + len) else {
            break;
        };
        pos += len;
        if entry.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(entry);
        let (key, value) = match text.split_once('=') {
            Some((k, v)) => (k, v),
            None => (text.as_ref(), ""),
        };
        let key: String = key.chars().take(MAX_TXT_KEY).collect::<String>().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let value: String = value.chars().take(MAX_TXT_VALUE).collect();
        out.entry(key).or_insert(value);
    }
    out
}

/// Strip the trailing `.local` (and any trailing dot) from a name, for display.
pub fn strip_local(name: &str) -> &str {
    let name = name.trim_end_matches('.');
    name.strip_suffix(".local").unwrap_or(name)
}

/// Split a service instance name (`Office Printer._ipp._tcp.local`) into the
/// instance label and the service type.
///
/// mDNS escapes dots inside an instance label as `\.`, which is why this walks
/// the string rather than calling `split('.')`.
pub fn split_instance(full: &str) -> Option<(String, String)> {
    let bytes: Vec<char> = full.chars().collect();
    let mut i = 0usize;
    let mut instance = String::new();
    while i < bytes.len() {
        match bytes[i] {
            '\\' if i + 1 < bytes.len() => {
                instance.push(bytes[i + 1]);
                i += 2;
            }
            '.' => {
                let rest: String = bytes[i + 1..].iter().collect();
                if rest.starts_with('_') {
                    return Some((instance, rest));
                }
                instance.push('.');
                i += 1;
            }
            c => {
                instance.push(c);
                i += 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a response packet from parts, so tests can express malformed input
    /// precisely instead of hand-writing hex.
    struct Builder {
        buf: Vec<u8>,
        answers: u16,
    }

    impl Builder {
        fn new() -> Self {
            Builder {
                buf: vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],
                answers: 0,
            }
        }

        fn name(mut self, name: &str) -> Self {
            encode_name(name, &mut self.buf);
            self
        }

        fn raw(mut self, bytes: &[u8]) -> Self {
            self.buf.extend_from_slice(bytes);
            self
        }

        fn record(mut self, name: &str, rtype: u16, ttl: u32, body: &[u8]) -> Self {
            encode_name(name, &mut self.buf);
            self.buf.extend_from_slice(&rtype.to_be_bytes());
            self.buf.extend_from_slice(&CLASS_IN.to_be_bytes());
            self.buf.extend_from_slice(&ttl.to_be_bytes());
            self.buf
                .extend_from_slice(&(body.len() as u16).to_be_bytes());
            self.buf.extend_from_slice(body);
            self.answers += 1;
            self
        }

        fn finish(mut self) -> Vec<u8> {
            let count = self.answers.to_be_bytes();
            self.buf[6] = count[0];
            self.buf[7] = count[1];
            self.buf
        }
    }

    fn txt_body(entries: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for entry in entries {
            out.push(entry.len() as u8);
            out.extend_from_slice(entry.as_bytes());
        }
        out
    }

    #[test]
    fn a_query_asks_for_a_unicast_reply_and_encodes_its_labels() {
        let packet = build_query(&["_services._dns-sd._udp.local"]);
        assert_eq!(&packet[0..2], &[0, 0], "no transaction id");
        assert_eq!(&packet[4..6], &[0, 1], "one question");
        // The class carries the unicast-response bit.
        let class = u16::from_be_bytes([packet[packet.len() - 2], packet[packet.len() - 1]]);
        assert_eq!(class, CLASS_IN | UNICAST_RESPONSE);
        // Labels are length-prefixed, so the dotted form never appears raw.
        assert!(packet.windows(9).any(|w| w == b"_services"));
        assert!(!packet.windows(2).any(|w| w == b".."));
    }

    #[test]
    fn an_instance_query_asks_for_every_record_type_at_once() {
        let packet = build_instance_query(&["Printer._ipp._tcp.local".to_string()]);
        assert_eq!(&packet[4..6], &[0, 1]);
        let rtype = u16::from_be_bytes([packet[packet.len() - 4], packet[packet.len() - 3]]);
        assert_eq!(rtype, TYPE_ANY);
    }

    #[test]
    fn parses_ptr_srv_txt_a_and_aaaa() {
        let packet = Builder::new()
            .record(
                "_ipp._tcp.local",
                TYPE_PTR,
                4500,
                &{
                    let mut b = Vec::new();
                    encode_name("Office Printer._ipp._tcp.local", &mut b);
                    b
                },
            )
            .record("Office Printer._ipp._tcp.local", TYPE_SRV, 120, &{
                let mut b = vec![0, 0, 0, 0, 0x02, 0x77]; // prio, weight, port 631
                encode_name("printer.local", &mut b);
                b
            })
            .record(
                "Office Printer._ipp._tcp.local",
                TYPE_TXT,
                4500,
                &txt_body(&["ty=Acme LaserFast 400", "usb_MDL=LaserFast 400", "flag"]),
            )
            .record("printer.local", TYPE_A, 120, &[192, 0, 2, 40])
            .record(
                "printer.local",
                TYPE_AAAA,
                120,
                &[
                    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x28,
                ],
            )
            .finish();

        let msg = parse(&packet).expect("packet parses");
        assert_eq!(msg.answers.len(), 5);

        assert_eq!(
            msg.answers[0].data,
            RecordData::Ptr("Office Printer._ipp._tcp.local".into())
        );
        match &msg.answers[1].data {
            RecordData::Srv { port, target } => {
                assert_eq!(*port, 631);
                assert_eq!(target, "printer.local");
            }
            other => panic!("expected SRV, got {other:?}"),
        }
        match &msg.answers[2].data {
            RecordData::Txt(map) => {
                assert_eq!(map.get("ty").map(String::as_str), Some("Acme LaserFast 400"));
                assert_eq!(map.get("usb_mdl").map(String::as_str), Some("LaserFast 400"));
                // A bare flag with no `=` is kept with an empty value.
                assert_eq!(map.get("flag").map(String::as_str), Some(""));
            }
            other => panic!("expected TXT, got {other:?}"),
        }
        assert_eq!(msg.answers[3].data, RecordData::A("192.0.2.40".parse().unwrap()));
        assert_eq!(
            msg.answers[4].data,
            RecordData::Aaaa("2001:db8::28".parse().unwrap())
        );
        assert_eq!(msg.answers[0].ttl, 4500);
    }

    #[test]
    fn follows_a_backward_compression_pointer() {
        // "printer.local" written once, then referenced by a pointer from the
        // SRV target — the ordinary case every responder produces.
        let mut buf = Builder::new().record("printer.local", TYPE_A, 120, &[192, 0, 2, 40]);
        let target_offset = 12u16; // the first name in the packet
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x90]; // port 8080
        srv.extend_from_slice(&(0xC000u16 | target_offset).to_be_bytes());
        buf = buf.record("Thing._http._tcp.local", TYPE_SRV, 120, &srv);
        let msg = parse(&buf.finish()).unwrap();
        match &msg.answers[1].data {
            RecordData::Srv { port, target } => {
                assert_eq!(*port, 8080);
                assert_eq!(target, "printer.local");
            }
            other => panic!("expected SRV, got {other:?}"),
        }
    }

    #[test]
    fn a_pointer_loop_is_refused_rather_than_hanging() {
        // A name at offset 12 whose only content is a pointer back to itself.
        let mut buf = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        buf.extend_from_slice(&(0xC000u16 | 12).to_be_bytes());
        buf.extend_from_slice(&TYPE_A.to_be_bytes());
        buf.extend_from_slice(&CLASS_IN.to_be_bytes());
        buf.extend_from_slice(&120u32.to_be_bytes());
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&[192, 0, 2, 1]);

        let msg = parse(&buf).unwrap();
        assert!(msg.answers.is_empty(), "a self-referential name yields nothing");
    }

    #[test]
    fn a_forward_pointer_is_refused() {
        let mut buf = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        // Points forwards, which no legitimate encoder produces and which is the
        // shape a mutual-reference loop needs.
        buf.extend_from_slice(&(0xC000u16 | 40).to_be_bytes());
        buf.resize(64, 0);
        let msg = parse(&buf).unwrap();
        assert!(msg.answers.is_empty());
    }

    #[test]
    fn mutually_referencing_pointers_terminate() {
        // Two pointers that point at each other: 12 -> 14, 14 -> 12.
        let mut buf = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        buf.extend_from_slice(&(0xC000u16 | 14).to_be_bytes());
        buf.extend_from_slice(&(0xC000u16 | 12).to_be_bytes());
        buf.resize(64, 0);
        assert!(parse(&buf).unwrap().answers.is_empty());
    }

    #[test]
    fn an_oversized_packet_is_refused_outright() {
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert!(parse(&big).is_none());
    }

    #[test]
    fn a_runt_packet_is_refused() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0x00; 11]).is_none());
    }

    #[test]
    fn a_query_packet_is_not_treated_as_a_response() {
        let query = build_query(&["_ipp._tcp.local"]);
        assert!(parse(&query).is_none());
    }

    #[test]
    fn a_truncated_record_keeps_the_records_before_it() {
        let mut packet = Builder::new()
            .record("printer.local", TYPE_A, 120, &[192, 0, 2, 40])
            .record("other.local", TYPE_A, 120, &[192, 0, 2, 41])
            .finish();
        packet.truncate(packet.len() - 3);
        let msg = parse(&packet).unwrap();
        assert_eq!(msg.answers.len(), 1);
    }

    #[test]
    fn a_record_claiming_more_data_than_the_packet_holds_is_dropped() {
        let mut packet = Builder::new().finish();
        // Header says one answer; the record declares a 60000-byte body.
        packet[6] = 0;
        packet[7] = 1;
        encode_name("a.local", &mut packet);
        packet.extend_from_slice(&TYPE_TXT.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());
        packet.extend_from_slice(&120u32.to_be_bytes());
        packet.extend_from_slice(&60_000u16.to_be_bytes());
        packet.extend_from_slice(b"short");
        assert!(parse(&packet).unwrap().answers.is_empty());
    }

    #[test]
    fn a_declared_record_count_far_beyond_the_packet_costs_nothing() {
        let mut packet = Builder::new().finish();
        packet[6] = 0xFF;
        packet[7] = 0xFF;
        let msg = parse(&packet).unwrap();
        assert!(msg.answers.is_empty());
    }

    #[test]
    fn record_parsing_stops_at_the_record_limit() {
        let mut builder = Builder::new();
        for i in 0..(MAX_RECORDS + 40) {
            builder = builder.record("a.local", TYPE_A, 120, &[192, 0, 2, (i % 250) as u8]);
        }
        let msg = parse(&builder.finish()).unwrap();
        assert_eq!(msg.answers.len(), MAX_RECORDS);
    }

    #[test]
    fn an_over_long_label_is_refused() {
        // A label length byte of 200 is illegal (max 63) and must not be read.
        let mut packet = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        packet.push(200);
        packet.extend_from_slice(&[b'a'; 200]);
        packet.push(0);
        packet.extend_from_slice(&TYPE_A.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());
        packet.extend_from_slice(&120u32.to_be_bytes());
        packet.extend_from_slice(&4u16.to_be_bytes());
        packet.extend_from_slice(&[192, 0, 2, 1]);
        assert!(parse(&packet).unwrap().answers.is_empty());
    }

    #[test]
    fn a_name_longer_than_the_dns_limit_is_refused() {
        let mut packet = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        // Ten 60-byte labels is 610 bytes of name, past the 255-byte limit.
        for _ in 0..10 {
            packet.push(60);
            packet.extend_from_slice(&[b'x'; 60]);
        }
        packet.push(0);
        packet.extend_from_slice(&TYPE_A.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());
        packet.extend_from_slice(&120u32.to_be_bytes());
        packet.extend_from_slice(&4u16.to_be_bytes());
        packet.extend_from_slice(&[192, 0, 2, 1]);
        assert!(parse(&packet).unwrap().answers.is_empty());
    }

    #[test]
    fn invalid_utf8_in_a_name_is_replaced_not_fatal() {
        let mut packet = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        packet.push(3);
        packet.extend_from_slice(&[0xFF, 0xFE, b'a']);
        packet.push(0);
        packet.extend_from_slice(&TYPE_A.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());
        packet.extend_from_slice(&120u32.to_be_bytes());
        packet.extend_from_slice(&4u16.to_be_bytes());
        packet.extend_from_slice(&[192, 0, 2, 1]);
        let msg = parse(&packet).unwrap();
        assert_eq!(msg.answers.len(), 1);
        assert!(msg.answers[0].name.contains('\u{FFFD}'));
    }

    #[test]
    fn txt_records_are_bounded_in_count_key_and_value() {
        let mut entries: Vec<String> = Vec::new();
        for i in 0..(MAX_TXT_PAIRS + 20) {
            entries.push(format!("k{i}=v{i}"));
        }
        entries.push(format!("{}=x", "K".repeat(MAX_TXT_KEY * 3)));
        entries.push(format!("big={}", "y".repeat(MAX_TXT_VALUE * 3)));
        let refs: Vec<&str> = entries.iter().map(String::as_str).collect();

        let packet = Builder::new()
            .record("a._http._tcp.local", TYPE_TXT, 120, &txt_body(&refs))
            .finish();
        let msg = parse(&packet).unwrap();
        match &msg.answers[0].data {
            RecordData::Txt(map) => {
                assert_eq!(map.len(), MAX_TXT_PAIRS);
                for (key, value) in map {
                    assert!(key.chars().count() <= MAX_TXT_KEY);
                    assert!(value.chars().count() <= MAX_TXT_VALUE);
                }
            }
            other => panic!("expected TXT, got {other:?}"),
        }
    }

    #[test]
    fn a_duplicate_txt_key_keeps_the_first_value() {
        let packet = Builder::new()
            .record(
                "a._http._tcp.local",
                TYPE_TXT,
                120,
                &txt_body(&["ty=First", "ty=Second"]),
            )
            .finish();
        let msg = parse(&packet).unwrap();
        match &msg.answers[0].data {
            RecordData::Txt(map) => assert_eq!(map.get("ty").map(String::as_str), Some("First")),
            other => panic!("expected TXT, got {other:?}"),
        }
    }

    #[test]
    fn a_txt_entry_running_past_the_record_stops_parsing_cleanly() {
        // Length byte says 50, only 3 bytes follow.
        let packet = Builder::new()
            .record("a._http._tcp.local", TYPE_TXT, 120, &[50, b'a', b'b', b'c'])
            .finish();
        let msg = parse(&packet).unwrap();
        match &msg.answers[0].data {
            RecordData::Txt(map) => assert!(map.is_empty()),
            other => panic!("expected TXT, got {other:?}"),
        }
    }

    #[test]
    fn a_record_of_an_unknown_type_is_skipped_by_its_length() {
        let packet = Builder::new()
            .record("a.local", 99, 120, &[1, 2, 3, 4, 5])
            .record("b.local", TYPE_A, 120, &[192, 0, 2, 7])
            .finish();
        let msg = parse(&packet).unwrap();
        assert_eq!(msg.answers.len(), 2);
        assert_eq!(msg.answers[0].data, RecordData::Other);
        assert_eq!(msg.answers[1].data, RecordData::A("192.0.2.7".parse().unwrap()));
    }

    #[test]
    fn an_a_record_with_the_wrong_length_is_not_decoded_as_an_address() {
        let packet = Builder::new()
            .record("a.local", TYPE_A, 120, &[192, 0])
            .finish();
        assert_eq!(parse(&packet).unwrap().answers[0].data, RecordData::Other);
    }

    #[test]
    fn questions_are_skipped_before_the_answers_are_read() {
        let mut builder = Builder::new().raw(&[]);
        // Hand-build: one question then one answer.
        builder.buf[4] = 0;
        builder.buf[5] = 1;
        builder = builder.name("_ipp._tcp.local");
        builder = builder.raw(&TYPE_PTR.to_be_bytes());
        builder = builder.raw(&CLASS_IN.to_be_bytes());
        let packet = builder
            .record("printer.local", TYPE_A, 120, &[192, 0, 2, 40])
            .finish();
        let msg = parse(&packet).unwrap();
        assert_eq!(msg.answers.len(), 1);
        assert_eq!(msg.answers[0].name, "printer.local");
    }

    #[test]
    fn splits_instance_names_including_escaped_dots() {
        assert_eq!(
            split_instance("Office Printer._ipp._tcp.local"),
            Some(("Office Printer".into(), "_ipp._tcp.local".into()))
        );
        assert_eq!(
            split_instance("Sam\\.s Mac._device-info._tcp.local"),
            Some(("Sam.s Mac".into(), "_device-info._tcp.local".into()))
        );
        assert_eq!(split_instance("printer.local"), None);
    }

    #[test]
    fn strips_the_local_suffix_for_display() {
        assert_eq!(strip_local("printer.local"), "printer");
        assert_eq!(strip_local("printer.local."), "printer");
        assert_eq!(strip_local("printer"), "printer");
        assert_eq!(strip_local("my.local.printer"), "my.local.printer");
    }

    #[test]
    fn parsing_never_panics_on_arbitrary_bytes() {
        // A deterministic sweep of hostile-looking packets: every byte value in
        // the length position, pointer bytes everywhere, and random-ish noise.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for size in [12usize, 13, 20, 64, 256, 1024] {
            for _ in 0..500 {
                let mut packet: Vec<u8> = (0..size).map(|_| (next() & 0xFF) as u8).collect();
                packet[2] = 0x84; // force the response bit so parsing proceeds
                packet[3] = 0x00;
                let _ = parse(&packet);
            }
        }
        for byte in 0u16..=255 {
            let mut packet = vec![0x00, 0x00, 0x84, 0x00, 0, 0, 0, 4, 0, 0, 0, 0];
            packet.resize(64, byte as u8);
            let _ = parse(&packet);
        }
    }
}
