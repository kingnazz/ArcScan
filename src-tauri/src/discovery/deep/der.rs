//! Just enough DER to read a certificate's subject.
//!
//! # Why this exists rather than a dependency
//!
//! ArcScan needs four strings out of an X.509 certificate: the subject's common
//! name and organisation, and the DNS names in the subject-alternative-name
//! extension. A full X.509 library validates chains, checks signatures, parses
//! every extension and understands a decade of encoding quirks — all of which
//! is code executing on bytes handed over by an unauthenticated device, in
//! service of filling in a "Model" column.
//!
//! So this reads the structure and nothing else. It verifies no signature,
//! trusts no certificate, and makes no security decision of any kind; the
//! result is display text. Every read is bounds-checked and there is no
//! recursion, so a malformed or hostile certificate produces `None` rather than
//! a panic or a stack overflow.

/// One tag-length-value triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    /// The contents, excluding the tag and length bytes.
    pub value: &'a [u8],
    /// Where the next triple starts, relative to the slice `read` was given.
    pub end: usize,
}

/// Read one TLV starting at `pos`.
///
/// Refuses indefinite-length encodings (which DER forbids) and any length that
/// would run past the end of the buffer.
pub fn read(bytes: &[u8], pos: usize) -> Option<Tlv<'_>> {
    let tag = *bytes.get(pos)?;
    let first_len = *bytes.get(pos + 1)?;
    let (length, header) = if first_len & 0x80 == 0 {
        (first_len as usize, 2)
    } else {
        let count = (first_len & 0x7F) as usize;
        // 0x80 is the indefinite form: legal in BER, forbidden in DER, and not
        // something a certificate may use here. More than four length bytes is
        // a certificate larger than any this will ever read.
        if count == 0 || count > 4 {
            return None;
        }
        let mut length = 0usize;
        for i in 0..count {
            length = (length << 8) | *bytes.get(pos + 2 + i)? as usize;
        }
        (length, 2 + count)
    };
    let start = pos.checked_add(header)?;
    let end = start.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    Some(Tlv {
        tag,
        value: bytes.get(start..end)?,
        end,
    })
}

/// Every TLV directly inside `bytes`, in order.
///
/// Bounded by `MAX_CHILDREN` so a certificate claiming a million empty elements
/// costs nothing.
pub fn children(bytes: &[u8]) -> Vec<Tlv<'_>> {
    const MAX_CHILDREN: usize = 256;
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() && out.len() < MAX_CHILDREN {
        let Some(tlv) = read(bytes, pos) else { break };
        // A zero-width element would not advance and would spin forever.
        if tlv.end <= pos {
            break;
        }
        pos = tlv.end;
        out.push(tlv);
    }
    out
}

pub const TAG_SEQUENCE: u8 = 0x30;
pub const TAG_SET: u8 = 0x31;
pub const TAG_OID: u8 = 0x06;
pub const TAG_OCTET_STRING: u8 = 0x04;
pub const TAG_BOOLEAN: u8 = 0x01;

/// Decode a DER string, whatever flavour it was encoded as.
///
/// PrintableString, UTF8String, IA5String and friends are all bytes that a
/// human is meant to read. T61/Teletex is treated as Latin-1, which is what it
/// is in practice. The result is lossy rather than refused: a subject with one
/// odd byte in it is still worth showing.
pub fn decode_string(tag: u8, bytes: &[u8]) -> Option<String> {
    let text = match tag {
        // UTF8String, PrintableString, IA5String, VisibleString, NumericString,
        // GeneralString, UniversalString-as-bytes.
        0x0C | 0x13 | 0x16 | 0x1A | 0x12 | 0x1B => String::from_utf8_lossy(bytes).into_owned(),
        // BMPString is UTF-16BE.
        0x1E => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        // T61String, in practice Latin-1.
        0x14 => bytes.iter().map(|b| *b as char).collect(),
        _ => return None,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_form_length_reads() {
        // SEQUENCE of three bytes.
        let bytes = [0x30, 0x03, 0x01, 0x02, 0x03];
        let tlv = read(&bytes, 0).unwrap();
        assert_eq!(tlv.tag, TAG_SEQUENCE);
        assert_eq!(tlv.value, &[0x01, 0x02, 0x03]);
        assert_eq!(tlv.end, 5);
    }

    #[test]
    fn a_long_form_length_reads() {
        let mut bytes = vec![0x04, 0x82, 0x01, 0x00];
        bytes.extend(std::iter::repeat_n(0xAA, 256));
        let tlv = read(&bytes, 0).unwrap();
        assert_eq!(tlv.value.len(), 256);
        assert_eq!(tlv.end, 260);
    }

    #[test]
    fn a_length_running_past_the_buffer_is_refused() {
        assert!(read(&[0x30, 0x10, 0x01], 0).is_none());
        assert!(read(&[0x30], 0).is_none());
        assert!(read(&[], 0).is_none());
    }

    #[test]
    fn the_indefinite_form_is_refused() {
        // Legal BER, forbidden DER, and a way to make a parser hunt for an
        // end-of-contents marker that may never come.
        assert!(read(&[0x30, 0x80, 0x00, 0x00], 0).is_none());
    }

    #[test]
    fn an_absurd_length_prefix_is_refused() {
        assert!(read(&[0x30, 0x88, 1, 2, 3, 4, 5, 6, 7, 8], 0).is_none());
    }

    #[test]
    fn children_are_walked_in_order_and_stop_at_damage() {
        let bytes = [0x02, 0x01, 0x05, 0x02, 0x01, 0x06, 0x30, 0x40];
        let found = children(&bytes);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].value, &[0x05]);
        assert_eq!(found[1].value, &[0x06]);
    }

    #[test]
    fn walking_hostile_bytes_terminates() {
        // The property that matters: whatever the bytes say, this returns.
        for pattern in [vec![0xFFu8; 512], vec![0x30; 512], vec![0x00; 512]] {
            let _ = children(&pattern);
        }
    }

    #[test]
    fn strings_decode_from_every_flavour_certificates_use() {
        assert_eq!(
            decode_string(0x13, b"DiskStation").as_deref(),
            Some("DiskStation")
        );
        assert_eq!(
            decode_string(0x0C, "iDRAC-7SZ1B43".as_bytes()).as_deref(),
            Some("iDRAC-7SZ1B43")
        );
        // BMPString: "Hi" in UTF-16BE.
        assert_eq!(
            decode_string(0x1E, &[0x00, 0x48, 0x00, 0x69]).as_deref(),
            Some("Hi")
        );
    }

    #[test]
    fn a_blank_or_unknown_string_decodes_to_nothing() {
        assert_eq!(decode_string(0x13, b"   "), None);
        assert_eq!(decode_string(0x30, b"not a string"), None);
    }
}
