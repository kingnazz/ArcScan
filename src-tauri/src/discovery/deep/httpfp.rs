//! Reading identity out of an HTTP response.
//!
//! Deep scanning asks a device that is already serving HTTP for its front page
//! and reads four things: the status, the `Server` header, the realm of any
//! authentication challenge, and the document title. Between them they name a
//! surprising proportion of the appliances on a business network, because an
//! embedded web server is written by the manufacturer and says so.
//!
//! # What is not done here
//!
//! No credentials are sent, no form is submitted, no path beyond `/` is
//! requested, and a redirect is recorded rather than followed. The request is a
//! `GET /` an ordinary browser would make, and the response is read into a
//! bounded buffer. Nothing about this probe is an attempt to get past anything.

use std::collections::BTreeMap;

use crate::discovery::model::{
    sanitize_field, Confidence, DiscoverySource, Evidence, EvidenceKind,
};

use super::fingerprint::match_signature;

/// Most of a body worth scanning for a `<title>`. A title that has not appeared
/// in the first 64 KiB is not going to be useful.
pub const MAX_TITLE_SCAN_BYTES: usize = 64 * 1024;

/// What one HTTP response said about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpFingerprint {
    pub status: u16,
    /// Header names lowercased; values trimmed and sanitized.
    pub headers: BTreeMap<String, String>,
    /// The `<title>` of the document, when there is one.
    pub title: Option<String>,
}

impl HttpFingerprint {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn server(&self) -> Option<&str> {
        self.header("server")
    }

    /// The realm from a `WWW-Authenticate: Basic realm="iDRAC9"` challenge.
    ///
    /// Worth reading on its own: a device that demands authentication often
    /// names itself in the demand, and that is the only string a locked-down
    /// appliance offers at all.
    pub fn auth_realm(&self) -> Option<String> {
        let challenge = self.header("www-authenticate")?;
        let lower = challenge.to_lowercase();
        let start = lower.find("realm=")? + "realm=".len();
        let rest = challenge.get(start..)?.trim_start();
        let value = if let Some(stripped) = rest.strip_prefix('"') {
            stripped.split('"').next().unwrap_or_default()
        } else {
            rest.split(',').next().unwrap_or_default().trim()
        };
        sanitize_field(value)
    }
}

/// Parse a raw HTTP response.
///
/// Returns `None` for anything that is not an HTTP response, which is what a
/// device answering a different protocol on port 80 looks like.
pub fn parse_response(raw: &[u8]) -> Option<HttpFingerprint> {
    // Headers are ASCII by specification and the body may be anything, so the
    // whole buffer is read lossily rather than being rejected for one invalid
    // byte in a page ArcScan is not going to render.
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.split("\r\n");
    let status_line = lines.next()?;
    if !status_line.starts_with("HTTP/") {
        return None;
    }
    let status = status_line.split_whitespace().nth(1)?.parse::<u16>().ok()?;

    let mut headers = BTreeMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(value) = sanitize_field(value) {
            // First header wins, so a device repeating a header cannot grow the
            // map without bound.
            headers.entry(name).or_insert(value);
        }
    }

    // The body starts after the blank line. Splitting the original text rather
    // than rejoining the iterator keeps the offsets right when a header value
    // itself contained a colon.
    let title = text
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| extract_title(body));

    Some(HttpFingerprint {
        status,
        headers,
        title,
    })
}

/// Pull the `<title>` out of a document.
///
/// A deliberately small reader rather than an HTML parser: the only element
/// wanted is one whose content is plain text by definition, and adding a parser
/// to read it would be adding an attack surface to read a string.
pub fn extract_title(body: &str) -> Option<String> {
    let window = &body[..body.len().min(MAX_TITLE_SCAN_BYTES)];
    let lower = window.to_lowercase();
    let open = lower.find("<title")?;
    // Skip any attributes on the tag itself.
    let after_open = window.get(open..)?.find('>')? + open + 1;
    let close = lower.get(after_open..)?.find("</title>")? + after_open;
    let raw = window.get(after_open..close)?;
    sanitize_field(&decode_basic_entities(raw))
}

/// Decode the five entities that appear in real page titles.
///
/// Not a general entity decoder: the value is displayed as text and never
/// interpreted, so the only reason to decode at all is that `AT&amp;T` should
/// read as `AT&T` in the drawer.
fn decode_basic_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Turn a fingerprint into discovery evidence.
///
/// The `Server` header and the title are recorded verbatim whether or not they
/// match a signature: an unrecognised appliance that says `Server: ACME
/// Controller 4.1` has still told a technician more than ArcScan knew before,
/// and the point of a deep scan is to write that down.
pub fn evidence(fingerprint: &HttpFingerprint, port: u16) -> Vec<Evidence> {
    let mut out = Vec::new();
    let key = port.to_string();

    if let Some(server) = fingerprint.server() {
        out.push(Evidence::new(
            DiscoverySource::Http,
            EvidenceKind::Banner,
            format!("http-server:{key}"),
            server,
            Confidence::Medium,
        ));
    }
    if let Some(title) = &fingerprint.title {
        out.push(Evidence::new(
            DiscoverySource::Http,
            EvidenceKind::PageTitle,
            &key,
            title,
            Confidence::Medium,
        ));
    }
    if let Some(realm) = fingerprint.auth_realm() {
        out.push(Evidence::new(
            DiscoverySource::Http,
            EvidenceKind::Banner,
            format!("http-realm:{key}"),
            &realm,
            Confidence::Medium,
        ));
    }

    // Identity from whichever of the three strings a signature recognises.
    // Checked in order of how deliberate each one is: a `Server` header is
    // written by the firmware author, a realm is written for a login prompt,
    // and a title is written for a person.
    for candidate in [
        fingerprint.server().map(str::to_string),
        fingerprint.auth_realm(),
        fingerprint.title.clone(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(signature) = match_signature(&candidate) else {
            continue;
        };
        if let Some(manufacturer) = signature.manufacturer {
            out.push(Evidence::new(
                DiscoverySource::Http,
                EvidenceKind::Manufacturer,
                "",
                manufacturer,
                signature.confidence,
            ));
        }
        if let Some(model) = signature.model {
            out.push(Evidence::new(
                DiscoverySource::Http,
                EvidenceKind::Model,
                "",
                model,
                signature.confidence,
            ));
        }
        if let Some(family) = signature.os_family {
            out.push(Evidence::new(
                DiscoverySource::Http,
                EvidenceKind::OsFamily,
                "",
                family,
                signature.confidence,
            ));
        }
        // One signature is enough. A second match on a weaker string would add
        // a duplicate claim, not a corroborating one.
        break;
    }

    out
}

/// The request deep scanning sends. A plain `GET /`, `Connection: close`, and
/// nothing that identifies the operator.
pub fn request_bytes(host: &str) -> Vec<u8> {
    format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: ArcScan\r\nAccept: */*\r\n\
         Connection: close\r\n\r\n"
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDRAC: &[u8] = b"HTTP/1.1 401 Unauthorized\r\n\
Server: Mbedthis-Appweb/2.4.2\r\n\
WWW-Authenticate: Basic realm=\"iDRAC9\"\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><head><title>iDRAC9</title></head><body></body></html>";

    const CANON: &[u8] = b"HTTP/1.1 200 OK\r\n\
Server: Canon HTTP Server\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><head><title>imageRUNNER ADVANCE C5535i</title></head></html>";

    const IIS: &[u8] = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/html\r\n\
Server: Microsoft-IIS/10.0\r\n\
X-Powered-By: ASP.NET\r\n\
\r\n\
<html><head><title>Intranet</title></head></html>";

    #[test]
    fn a_status_line_and_headers_parse() {
        let parsed = parse_response(IIS).unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.server(), Some("Microsoft-IIS/10.0"));
        assert_eq!(parsed.header("x-powered-by"), Some("ASP.NET"));
        assert_eq!(parsed.title.as_deref(), Some("Intranet"));
    }

    #[test]
    fn header_names_are_matched_without_regard_to_case() {
        let raw = b"HTTP/1.1 200 OK\r\nSERVER: nginx\r\n\r\n";
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.server(), Some("nginx"));
    }

    #[test]
    fn a_non_http_reply_parses_to_nothing() {
        assert!(parse_response(b"SSH-2.0-OpenSSH_9.6\r\n").is_none());
        assert!(parse_response(b"").is_none());
        assert!(parse_response(b"\x00\x01\x02\x03").is_none());
    }

    #[test]
    fn an_authentication_realm_is_read_out_of_the_challenge() {
        let parsed = parse_response(IDRAC).unwrap();
        assert_eq!(parsed.status, 401);
        assert_eq!(parsed.auth_realm().as_deref(), Some("iDRAC9"));
    }

    #[test]
    fn an_unquoted_realm_still_reads() {
        let raw = b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=NAS, charset=UTF-8\r\n\r\n";
        let parsed = parse_response(raw).unwrap();
        assert_eq!(parsed.auth_realm().as_deref(), Some("NAS"));
    }

    #[test]
    fn a_locked_down_idrac_is_still_identified_from_its_realm_alone() {
        // The case this probe exists for: a 401 with no body still names the
        // device, and an iDRAC must never be recorded as the server it manages.
        let parsed = parse_response(IDRAC).unwrap();
        let found = evidence(&parsed, 443);
        let manufacturer = found
            .iter()
            .find(|e| e.kind == EvidenceKind::Manufacturer)
            .unwrap();
        assert_eq!(manufacturer.value, "Dell");
        let model = found
            .iter()
            .find(|e| e.kind == EvidenceKind::Model)
            .unwrap();
        assert_eq!(model.value, "iDRAC");
    }

    #[test]
    fn a_canon_printer_is_identified_from_its_server_header() {
        let parsed = parse_response(CANON).unwrap();
        let found = evidence(&parsed, 80);
        assert_eq!(
            found
                .iter()
                .find(|e| e.kind == EvidenceKind::Manufacturer)
                .map(|e| e.value.as_str()),
            Some("Canon")
        );
        // And the raw header is kept whether or not it matched anything.
        assert!(found
            .iter()
            .any(|e| e.kind == EvidenceKind::Banner && e.value == "Canon HTTP Server"));
    }

    #[test]
    fn iis_records_windows_as_a_family_and_never_as_a_version() {
        let parsed = parse_response(IIS).unwrap();
        let found = evidence(&parsed, 80);
        let family = found
            .iter()
            .find(|e| e.kind == EvidenceKind::OsFamily)
            .unwrap();
        assert_eq!(family.value, "windows");
        // The "10.0" in Microsoft-IIS/10.0 is IIS's version. Reading it as the
        // operating system's is the mistake this asserts against.
        assert!(!found
            .iter()
            .any(|e| e.kind == EvidenceKind::OsVersion || e.kind == EvidenceKind::OsProduct));
    }

    #[test]
    fn an_unrecognised_appliance_still_has_its_banner_recorded() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: ACME Controller 4.1\r\n\r\n<title>ACME</title>";
        let parsed = parse_response(raw).unwrap();
        let found = evidence(&parsed, 80);
        assert!(found
            .iter()
            .any(|e| e.kind == EvidenceKind::Banner && e.value == "ACME Controller 4.1"));
        // Nothing was recognised, so nothing is claimed about what it is.
        assert!(!found.iter().any(|e| e.kind == EvidenceKind::Manufacturer));
    }

    #[test]
    fn a_title_with_attributes_and_entities_reads_correctly() {
        assert_eq!(
            extract_title("<html><title lang=\"en\">AT&amp;T Router</title>").as_deref(),
            Some("AT&T Router")
        );
    }

    #[test]
    fn a_body_with_no_title_yields_none() {
        assert_eq!(extract_title("<html><body>hello</body></html>"), None);
        assert_eq!(extract_title(""), None);
        // An unterminated title is not a title.
        assert_eq!(extract_title("<title>never closed"), None);
    }

    #[test]
    fn an_empty_title_is_not_recorded_as_a_blank_fact() {
        assert_eq!(extract_title("<title>   </title>"), None);
    }

    #[test]
    fn control_characters_in_a_header_are_stripped_before_storage() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: bad\x07\x08value\r\n\r\n";
        let parsed = parse_response(raw).unwrap();
        let server = parsed.server().unwrap();
        assert!(!server.chars().any(char::is_control));
    }

    #[test]
    fn the_request_sends_no_credential_and_asks_for_nothing_but_the_root() {
        let request = String::from_utf8(request_bytes("10.0.0.5")).unwrap();
        assert!(request.starts_with("GET / HTTP/1.1\r\n"));
        assert!(request.contains("Connection: close"));
        let lower = request.to_lowercase();
        assert!(!lower.contains("authorization"));
        assert!(!lower.contains("cookie"));
    }
}
