//! Decode SNMP DisplayString / OctetString values into printable UTF-8.
//!
//! SNMP agents (Netgear especially) return ifName/ifAlias/ifDescr as
//! DisplayString in the RFC sense — NVT ASCII — or as whatever the web UI's
//! code page happened to be. Treating those bytes as UTF-8 with
//! `from_utf8_lossy` inserts U+FFFD replacement characters, which is the
//! mojibake reported against real Netgear hardware in issue #46.
//!
//! This decoder never returns a string containing U+FFFD. Unusable bytes fall
//! through to the next IF-MIB candidate or to the numeric ifIndex.

/// Logical Internet node id. Not an inventory device_id and never serialized
/// into the ArcAtlas inventory array.
pub const INTERNET_NODE_ID: &str = "logical:internet";

/// Decode an SNMP octet string into a printable UTF-8 label.
pub fn decode_snmp_display(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    // UTF-16-BE starts with a NUL; stripping leading padding would destroy it.
    if let Some(s) = decode_utf16_ascii(bytes) {
        return sanitize_label(&s);
    }
    let stripped = strip_trailing_padding(bytes);
    if stripped.is_empty() {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(stripped) {
        return sanitize_label(s);
    }
    sanitize_label(&decode_windows_1252(stripped))
}

/// True when the octets are not just empty padding.
pub fn is_nonempty_octets(bytes: &[u8]) -> bool {
    !strip_trailing_padding(bytes).is_empty()
}

/// Debug note for a rejected ifName/ifAlias/ifDescr. Hex only — never the
/// original secret-bearing payload of another MIB.
pub fn rejected_octets_note(field: &str, index: u32, bytes: &[u8]) -> String {
    let hex: String = strip_trailing_padding(bytes)
        .iter()
        .take(16)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{field} ifIndex {index} ignored non-printable octets: {hex}")
}

/// First usable printable candidate, in preference order.
pub fn first_printable<'a, I>(candidates: I) -> Option<String>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    for candidate in candidates {
        if let Some(s) = candidate.and_then(sanitize_label) {
            return Some(s);
        }
    }
    None
}

pub fn sanitize_label(s: &str) -> Option<String> {
    let trimmed = s
        .trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace())
        .to_string();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if trimmed.contains('\u{FFFD}') {
        return None;
    }
    let mut alnum = 0usize;
    let mut non_ascii = 0usize;
    let mut chars = 0usize;
    for ch in trimmed.chars() {
        chars += 1;
        if ch.is_control() {
            return None;
        }
        if !is_label_char(ch) {
            return None;
        }
        if ch.is_ascii_alphanumeric() || ch.is_alphanumeric() {
            alnum += 1;
        }
        if !ch.is_ascii() {
            non_ascii += 1;
        }
    }
    if alnum == 0 {
        return None;
    }
    // Port names and aliases are almost always ASCII. A short string that is
    // mostly high-bit Latin-1 is the Netgear mojibake case, not a real label.
    if non_ascii * 3 > chars {
        return None;
    }
    Some(trimmed)
}

fn is_label_char(ch: char) -> bool {
    if ch.is_ascii_graphic() || ch == ' ' {
        return true;
    }
    ch.is_alphanumeric()
}

fn strip_trailing_padding(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == 0 || bytes[end - 1].is_ascii_whitespace()) {
        end -= 1;
    }
    &bytes[..end]
}

/// UTF-16-BE/LE of an ASCII port name shows up as every other byte NUL.
fn decode_utf16_ascii(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let even_nul = bytes.iter().step_by(2).filter(|b| **b == 0).count();
    let odd_nul = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let pairs = bytes.len() / 2;
    if even_nul == pairs && odd_nul == 0 {
        let ascii: Vec<u8> = bytes.iter().skip(1).step_by(2).copied().collect();
        return std::str::from_utf8(&ascii)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if odd_nul == pairs && even_nul == 0 {
        let ascii: Vec<u8> = bytes.iter().step_by(2).copied().collect();
        return std::str::from_utf8(&ascii)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    None
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| windows_1252_char(b)).collect()
}

fn windows_1252_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'ÿ',
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => '\u{FFFD}',
        other => other as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_port_names_pass_through() {
        assert_eq!(decode_snmp_display(b"Gi1/0/18").as_deref(), Some("Gi1/0/18"));
        assert_eq!(decode_snmp_display(b"g7").as_deref(), Some("g7"));
        assert_eq!(decode_snmp_display(b"Port 7").as_deref(), Some("Port 7"));
        assert_eq!(decode_snmp_display(b"X0").as_deref(), Some("X0"));
    }

    #[test]
    fn latin1_alias_is_recovered_when_mostly_ascii() {
        // "Cafe" + Latin-1 é. One accented letter in an otherwise ASCII alias.
        let bytes = b"Caf\xE9-uplink";
        assert_eq!(
            decode_snmp_display(bytes).as_deref(),
            Some("Café-uplink")
        );
    }

    #[test]
    fn netgear_replacement_garbage_is_rejected() {
        // Observed class of bug: invalid UTF-8 mixed with punctuation, which
        // from_utf8_lossy rendered as "�=ü)". Must never become a port label.
        let lossy_shape = [0x80, b'=', 0xC3, 0xBC, b')'];
        assert_eq!(decode_snmp_display(&lossy_shape), None);
        let latin1_junk = [0x80, b'=', 0xFC, b')'];
        assert_eq!(decode_snmp_display(&latin1_junk), None);
        assert!(!decode_snmp_display(&lossy_shape)
            .unwrap_or_default()
            .contains('\u{FFFD}'));
    }

    #[test]
    fn replacement_character_is_never_returned() {
        let lossy = String::from_utf8_lossy(&[0xFF, 0xFE, b'g', b'7']);
        assert!(lossy.contains('\u{FFFD}'));
        assert_eq!(decode_snmp_display(&[0xFF, 0xFE, b'g', b'7']), None);
        assert_eq!(sanitize_label("Port \u{FFFD}7"), None);
    }

    #[test]
    fn nul_padded_and_utf16_ascii_port_names() {
        assert_eq!(
            decode_snmp_display(b"Gi1/0/18\0\0\0").as_deref(),
            Some("Gi1/0/18")
        );
        let utf16be = [0x00, b'g', 0x00, b'7'];
        assert_eq!(decode_snmp_display(&utf16be).as_deref(), Some("g7"));
        let utf16le = [b'g', 0x00, b'7', 0x00];
        assert_eq!(decode_snmp_display(&utf16le).as_deref(), Some("g7"));
    }

    #[test]
    fn empty_and_whitespace_are_none() {
        assert_eq!(decode_snmp_display(b""), None);
        assert_eq!(decode_snmp_display(b"   "), None);
        assert_eq!(decode_snmp_display(&[0, 0, 0]), None);
    }

    #[test]
    fn first_printable_skips_garbage() {
        assert_eq!(
            first_printable([Some("\u{FFFD}=ü)"), Some("g7"), Some("Uplink")]).as_deref(),
            Some("g7")
        );
        assert_eq!(
            first_printable([None, None, Some("12")]).as_deref(),
            Some("12")
        );
    }
}
