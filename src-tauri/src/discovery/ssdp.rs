//! SSDP: M-SEARCH construction and bounded response-header parsing.
//!
//! An SSDP response is an HTTP-shaped block of headers over UDP. That makes it
//! easy to read and easy to be careless with, so everything here is capped: the
//! response size, the header count, the length of a header name and of its
//! value. A line that does not parse is skipped rather than failing the
//! response, because a single vendor quirk should not lose a real device.
//!
//! Nothing in this module opens a connection. The `LOCATION` header it returns
//! is untrusted input and is only ever acted on after
//! [`super::urlguard`] has approved it.

use std::collections::BTreeMap;

/// Largest SSDP response accepted. Real responses are a few hundred bytes;
/// this is generous enough for a verbose vendor and small enough that a flood
/// costs nothing.
pub const MAX_RESPONSE_BYTES: usize = 8_192;

/// Most headers kept from one response.
pub const MAX_HEADERS: usize = 32;

/// Longest header name and value kept, in characters.
pub const MAX_HEADER_NAME: usize = 64;
pub const MAX_HEADER_VALUE: usize = 512;

/// The search targets ArcScan asks for.
///
/// `ssdp:all` returns every service on every device, which is what makes the
/// device-type declarations (`InternetGatewayDevice`, `MediaRenderer`, and so
/// on) visible. `upnp:rootdevice` is asked separately because a handful of
/// devices answer it and ignore `ssdp:all`. Two targets is the whole strategy —
/// anything more is redundant traffic for the same answers.
pub const SEARCH_TARGETS: [&str; 2] = ["ssdp:all", "upnp:rootdevice"];

/// The `MX` value: how many seconds a device may wait before answering, so a
/// hundred devices do not all reply in the same millisecond. Kept small because
/// the whole discovery window is only a few seconds.
pub const MX_SECONDS: u8 = 2;

/// One parsed SSDP response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    /// Header names lowercased; values trimmed. Duplicates keep the first.
    pub headers: BTreeMap<String, String>,
}

impl Response {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn location(&self) -> Option<&str> {
        self.get("location")
    }

    pub fn server(&self) -> Option<&str> {
        self.get("server")
    }

    pub fn search_target(&self) -> Option<&str> {
        self.get("st")
    }

    pub fn usn(&self) -> Option<&str> {
        self.get("usn")
    }

    /// The UDN embedded in a USN (`uuid:xxxx::urn:...`), which is the part that
    /// identifies the *device* rather than one of its services.
    ///
    /// Recorded as continuity evidence inside one network scope. It is never an
    /// identity key and never matched across scopes: a UDN is chosen by the
    /// device, and two networks can hold devices that were cloned from the same
    /// image and share one.
    pub fn udn(&self) -> Option<&str> {
        let usn = self.usn()?;
        let head = usn.split("::").next()?.trim();
        head.strip_prefix("uuid:")
            .filter(|rest| !rest.is_empty())
            .map(|_| head)
    }
}

/// Build an M-SEARCH datagram for one search target.
///
/// `MAN` must be quoted and `HOST` must be the multicast address literal; both
/// are required by the UPnP Device Architecture, and devices that validate
/// strictly ignore a request that gets either wrong.
pub fn build_msearch(target: &str) -> Vec<u8> {
    format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: 239.255.255.250:1900\r\n\
         MAN: \"ssdp:discover\"\r\n\
         MX: {MX_SECONDS}\r\n\
         ST: {target}\r\n\
         USER-AGENT: ArcScan/{version} UPnP/1.1\r\n\
         \r\n",
        version = env!("CARGO_PKG_VERSION"),
    )
    .into_bytes()
}

/// Parse an SSDP response.
///
/// Returns `None` when the datagram is oversized or is not a response at all —
/// notably an `M-SEARCH` or `NOTIFY` reflected back onto the socket, which
/// carries no answer and must not be counted as one.
pub fn parse(datagram: &[u8]) -> Option<Response> {
    if datagram.is_empty() || datagram.len() > MAX_RESPONSE_BYTES {
        return None;
    }
    // Header text is ASCII by specification; anything else is replaced rather
    // than rejected so one stray byte does not lose the device.
    let text = String::from_utf8_lossy(datagram);
    let mut lines = text.split(|c| c == '\n');

    let status = lines.next()?.trim_end_matches('\r').trim();
    if !status.to_ascii_uppercase().starts_with("HTTP/1.") {
        return None;
    }
    // "HTTP/1.1 200 OK" — anything else is a refusal, not a device.
    if !status.contains(" 200") {
        return None;
    }

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        if headers.len() >= MAX_HEADERS {
            break;
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            // A line with no colon is malformed. Skip it and keep reading: some
            // devices emit a stray continuation line mid-block.
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.chars().count() > MAX_HEADER_NAME {
            continue;
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        {
            continue;
        }
        let value: String = value.trim().chars().take(MAX_HEADER_VALUE).collect();
        // First value wins: a duplicated header is a device quirk, and choosing
        // deterministically keeps two scans of the same device identical.
        headers.entry(name).or_insert(value);
    }

    Some(Response { headers })
}

/// The device type declared by a search target or USN, reduced to its bare
/// name: `urn:schemas-upnp-org:device:MediaRenderer:1` becomes `MediaRenderer`.
pub fn urn_device_type(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let idx = lower.find(":device:")?;
    let rest = &value[idx + ":device:".len()..];
    let name = rest.split(':').next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &str) -> Option<Response> {
        parse(body.as_bytes())
    }

    #[test]
    fn an_msearch_is_specification_compliant() {
        let packet = String::from_utf8(build_msearch("ssdp:all")).unwrap();
        assert!(packet.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(packet.contains("HOST: 239.255.255.250:1900\r\n"));
        // MAN must be quoted; an unquoted one is ignored by strict devices.
        assert!(packet.contains("MAN: \"ssdp:discover\"\r\n"));
        assert!(packet.contains(&format!("MX: {MX_SECONDS}\r\n")));
        assert!(packet.contains("ST: ssdp:all\r\n"));
        assert!(packet.ends_with("\r\n\r\n"));
        // A small MX keeps the response window inside the discovery budget.
        assert!(MX_SECONDS <= 3);
    }

    #[test]
    fn header_names_are_case_insensitive_and_values_are_trimmed() {
        let r = response(
            "HTTP/1.1 200 OK\r\n\
             CACHE-CONTROL: max-age=1800\r\n\
             Location:   http://192.0.2.5:8080/desc.xml   \r\n\
             sErVeR: Linux/4.4 UPnP/1.0 Acme/2.1\r\n\
             ST: upnp:rootdevice\r\n\
             USN: uuid:9f8c1e4a-0000-4000-8000-0123456789ab::upnp:rootdevice\r\n\
             EXT:\r\n\
             \r\n",
        )
        .unwrap();
        assert_eq!(r.location(), Some("http://192.0.2.5:8080/desc.xml"));
        assert_eq!(r.server(), Some("Linux/4.4 UPnP/1.0 Acme/2.1"));
        assert_eq!(r.search_target(), Some("upnp:rootdevice"));
        assert_eq!(r.get("cache-control"), Some("max-age=1800"));
        assert_eq!(r.get("ext"), Some(""));
    }

    #[test]
    fn bare_newlines_and_missing_trailing_blank_line_still_parse() {
        let r = response("HTTP/1.1 200 OK\nLOCATION: http://192.0.2.5/d.xml\nST: ssdp:all").unwrap();
        assert_eq!(r.location(), Some("http://192.0.2.5/d.xml"));
    }

    #[test]
    fn the_udn_is_extracted_from_the_usn() {
        let r = response(
            "HTTP/1.1 200 OK\r\nUSN: uuid:11111111-2222-3333-4444-555555555555::urn:schemas-upnp-org:device:MediaRenderer:1\r\n",
        )
        .unwrap();
        assert_eq!(
            r.udn(),
            Some("uuid:11111111-2222-3333-4444-555555555555")
        );

        // A USN with no uuid prefix identifies nothing.
        let odd = response("HTTP/1.1 200 OK\r\nUSN: something-else::upnp:rootdevice\r\n").unwrap();
        assert_eq!(odd.udn(), None);
        // A bare `uuid:` with nothing after it is not an identifier either.
        let empty = response("HTTP/1.1 200 OK\r\nUSN: uuid:\r\n").unwrap();
        assert_eq!(empty.udn(), None);
    }

    #[test]
    fn a_reflected_search_or_notify_is_not_a_response() {
        assert!(response("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n").is_none());
        assert!(response("NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\n").is_none());
        assert!(response("garbage").is_none());
        assert!(response("").is_none());
    }

    #[test]
    fn a_non_200_status_is_refused() {
        assert!(response("HTTP/1.1 404 Not Found\r\nLOCATION: http://192.0.2.5/\r\n").is_none());
        assert!(response("HTTP/1.1 500 Server Error\r\n").is_none());
    }

    #[test]
    fn an_oversized_response_is_refused_outright() {
        let mut body = String::from("HTTP/1.1 200 OK\r\n");
        body.push_str(&"X-Pad: ".repeat(MAX_RESPONSE_BYTES));
        assert!(response(&body).is_none());
    }

    #[test]
    fn header_count_name_and_value_are_all_bounded() {
        let mut body = String::from("HTTP/1.1 200 OK\r\n");
        for i in 0..(MAX_HEADERS + 50) {
            body.push_str(&format!("X-Header-{i}: value{i}\r\n"));
        }
        body.push_str(&format!("{}: x\r\n", "N".repeat(MAX_HEADER_NAME * 2)));
        let r = response(&body).unwrap();
        assert_eq!(r.headers.len(), MAX_HEADERS);
        assert!(r.headers.keys().all(|k| k.chars().count() <= MAX_HEADER_NAME));

        let long_value = format!(
            "HTTP/1.1 200 OK\r\nSERVER: {}\r\n",
            "s".repeat(MAX_HEADER_VALUE * 3)
        );
        let r = response(&long_value).unwrap();
        assert_eq!(r.server().unwrap().chars().count(), MAX_HEADER_VALUE);
    }

    #[test]
    fn malformed_lines_are_skipped_without_losing_the_response() {
        let r = response(
            "HTTP/1.1 200 OK\r\n\
             this line has no colon\r\n\
             : empty name\r\n\
             Bad Header Name!: x\r\n\
             LOCATION: http://192.0.2.9/d.xml\r\n",
        )
        .unwrap();
        assert_eq!(r.location(), Some("http://192.0.2.9/d.xml"));
        assert_eq!(r.headers.len(), 1);
    }

    #[test]
    fn a_duplicated_header_keeps_the_first_value() {
        let r = response(
            "HTTP/1.1 200 OK\r\nLOCATION: http://192.0.2.1/a.xml\r\nlocation: http://192.0.2.2/b.xml\r\n",
        )
        .unwrap();
        assert_eq!(r.location(), Some("http://192.0.2.1/a.xml"));
    }

    #[test]
    fn a_response_with_no_location_parses_and_reports_none() {
        let r = response("HTTP/1.1 200 OK\r\nST: ssdp:all\r\n").unwrap();
        assert_eq!(r.location(), None);
    }

    #[test]
    fn invalid_utf8_does_not_lose_the_headers_around_it() {
        let mut bytes = b"HTTP/1.1 200 OK\r\nSERVER: ".to_vec();
        bytes.extend_from_slice(&[0xFF, 0xFE]);
        bytes.extend_from_slice(b"\r\nLOCATION: http://192.0.2.3/d.xml\r\n");
        let r = parse(&bytes).unwrap();
        assert_eq!(r.location(), Some("http://192.0.2.3/d.xml"));
        assert!(r.server().unwrap().contains('\u{FFFD}'));
    }

    #[test]
    fn device_types_are_read_out_of_a_urn() {
        assert_eq!(
            urn_device_type("urn:schemas-upnp-org:device:InternetGatewayDevice:1").as_deref(),
            Some("InternetGatewayDevice")
        );
        assert_eq!(
            urn_device_type("URN:SCHEMAS-UPNP-ORG:DEVICE:MediaRenderer:2").as_deref(),
            Some("MediaRenderer")
        );
        assert_eq!(urn_device_type("urn:schemas-upnp-org:service:AVTransport:1"), None);
        assert_eq!(urn_device_type("upnp:rootdevice"), None);
        assert_eq!(urn_device_type(""), None);
    }

    #[test]
    fn parsing_never_panics_on_arbitrary_bytes() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for size in [1usize, 16, 64, 512, 4096] {
            for _ in 0..400 {
                let packet: Vec<u8> = (0..size).map(|_| (next() & 0xFF) as u8).collect();
                let _ = parse(&packet);
                let mut prefixed = b"HTTP/1.1 200 OK\r\n".to_vec();
                prefixed.extend_from_slice(&packet);
                let _ = parse(&prefixed);
            }
        }
    }
}
