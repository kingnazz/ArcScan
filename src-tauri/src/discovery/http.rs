//! A single-purpose HTTP client: fetch one small XML document from one address
//! that [`super::urlguard`] has already approved.
//!
//! # Why not a real HTTP library
//!
//! Every general-purpose client is built to be *helpful*: it follows redirects,
//! honours proxy environment variables, re-resolves host names, retries, and
//! reads a response of whatever size arrives. Each of those is a hole in the
//! guarantees this release makes, and each would have to be switched off
//! individually and kept switched off. Forty lines of request-and-read has no
//! defaults to get wrong:
//!
//! * it connects to a [`SocketAddr`], so there is no name to resolve and no
//!   rebinding window between the check and the connection
//! * it never consults a proxy, so the request cannot be sent to a host that was
//!   never validated
//! * a `3xx` is an error, not an instruction
//! * the body is read into a fixed-size buffer and the connection is dropped the
//!   moment it is full
//! * `Connection: close` and HTTP/1.0 mean no keep-alive and no second request
//!
//! It also sends no cookies, no credentials and no authorization header, because
//! it has none to send.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::urlguard::ValidatedUrl;

/// How long the whole fetch may take, connection included.
pub const FETCH_TIMEOUT: Duration = Duration::from_millis(2_000);

/// How long the connection alone may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);

/// Most body bytes read. A description document is a few kilobytes; this is
/// generous and still bounded.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Most header bytes read before the body starts.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Connect,
    Timeout,
    Io,
    /// A redirect. Never followed: the destination was never validated.
    Redirect(u16),
    Status(u16),
    /// The response announced something that is not a device description.
    ContentType(String),
    TooLarge,
    NotHttp,
}

impl FetchError {
    pub fn reason(&self) -> String {
        match self {
            FetchError::Connect => "the device did not accept a connection".into(),
            FetchError::Timeout => "the device did not answer in time".into(),
            FetchError::Io => "the connection failed while reading".into(),
            FetchError::Redirect(code) => {
                format!("the device answered {code} with a redirect, which ArcScan does not follow")
            }
            FetchError::Status(code) => format!("the device answered {code}"),
            FetchError::ContentType(t) => format!("the device returned {t}, not a description"),
            FetchError::TooLarge => "the description was larger than ArcScan will read".into(),
            FetchError::NotHttp => "the reply was not an HTTP response".into(),
        }
    }
}

/// Fetch a device description.
///
/// The whole operation is wrapped in one deadline, so a device that accepts the
/// connection and then goes silent costs [`FETCH_TIMEOUT`] and nothing more.
pub async fn fetch_description(url: &ValidatedUrl) -> Result<String, FetchError> {
    match tokio::time::timeout(FETCH_TIMEOUT, fetch_inner(url)).await {
        Ok(result) => result,
        Err(_) => Err(FetchError::Timeout),
    }
}

async fn fetch_inner(url: &ValidatedUrl) -> Result<String, FetchError> {
    let addr: SocketAddr = url.addr;
    let mut stream = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(_)) => return Err(FetchError::Connect),
        Err(_) => return Err(FetchError::Timeout),
    };

    // HTTP/1.0 with an explicit close: no keep-alive to leave open, no chunked
    // transfer encoding to decode, and no second request on the socket.
    let request = format!(
        "GET {path} HTTP/1.0\r\n\
         Host: {host}\r\n\
         User-Agent: ArcScan/{version}\r\n\
         Accept: text/xml, application/xml\r\n\
         Connection: close\r\n\
         \r\n",
        path = url.path_and_query,
        host = url.host_header,
        version = env!("CARGO_PKG_VERSION"),
    );
    if stream.write_all(request.as_bytes()).await.is_err() {
        return Err(FetchError::Io);
    }

    let mut raw: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Err(FetchError::Io),
        };
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > MAX_HEADER_BYTES + MAX_BODY_BYTES {
            // Stop reading and drop the connection rather than draining a
            // device that has decided to send forever.
            return Err(FetchError::TooLarge);
        }
    }

    parse_response(&raw)
}

/// Split a raw HTTP response into its status, headers and body, applying the
/// status and content-type rules. Separated from the socket so it is testable.
pub fn parse_response(raw: &[u8]) -> Result<String, FetchError> {
    let (head_end, body_start) = find_header_end(raw).ok_or(FetchError::NotHttp)?;
    if head_end > MAX_HEADER_BYTES {
        return Err(FetchError::TooLarge);
    }
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let body_bytes = &raw[body_start..];

    let mut lines = head.split('\n');
    let status_line = lines.next().ok_or(FetchError::NotHttp)?.trim();
    if !status_line.to_ascii_uppercase().starts_with("HTTP/1.") {
        return Err(FetchError::NotHttp);
    }
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or(FetchError::NotHttp)?;
    if (300..400).contains(&code) {
        return Err(FetchError::Redirect(code));
    }
    if code != 200 {
        return Err(FetchError::Status(code));
    }

    // A description is XML. A device answering with HTML is serving its admin
    // page, not a description, and parsing it would be wasted work.
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-type") {
            continue;
        }
        let value = value.trim().to_ascii_lowercase();
        let mime = value.split(';').next().unwrap_or("").trim().to_string();
        if mime.is_empty() {
            break;
        }
        let acceptable =
            mime.contains("xml") || mime == "text/plain" || mime == "application/octet-stream";
        if !acceptable {
            return Err(FetchError::ContentType(mime));
        }
        break;
    }

    if body_bytes.len() > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge);
    }
    // Lossy on purpose: a device that mislabels its encoding should still yield
    // its friendly name rather than being discarded wholesale.
    Ok(String::from_utf8_lossy(body_bytes).into_owned())
}

/// Find the header/body boundary, accepting both `\r\n\r\n` and a bare
/// `\n\n` from a sloppy embedded server. Returns (header end, body start).
fn find_header_end(raw: &[u8]) -> Option<(usize, usize)> {
    for i in 0..raw.len() {
        if raw[i..].starts_with(b"\r\n\r\n") {
            return Some((i, i + 4));
        }
        if raw[i..].starts_with(b"\n\n") {
            return Some((i, i + 2));
        }
    }
    // A response with headers and no body still parses; the body is empty.
    raw.is_empty()
        .then_some((0, 0))
        .or(Some((raw.len(), raw.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(head: &str, body: &str) -> Vec<u8> {
        format!("{head}\r\n\r\n{body}").into_bytes()
    }

    #[test]
    fn a_normal_xml_response_yields_its_body() {
        let raw = response(
            "HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nContent-Length: 9",
            "<root/>\n",
        );
        assert_eq!(parse_response(&raw).unwrap(), "<root/>\n");
    }

    #[test]
    fn a_missing_content_type_is_accepted() {
        let raw = response("HTTP/1.0 200 OK\r\nServer: Acme", "<root/>");
        assert_eq!(parse_response(&raw).unwrap(), "<root/>");
    }

    #[test]
    fn every_redirect_is_refused_rather_than_followed() {
        for code in [301u16, 302, 303, 307, 308] {
            let raw = response(
                &format!("HTTP/1.1 {code} Moved\r\nLocation: http://evil.example/"),
                "",
            );
            assert_eq!(parse_response(&raw), Err(FetchError::Redirect(code)));
        }
    }

    #[test]
    fn an_error_status_is_reported_not_parsed() {
        for code in [400u16, 401, 403, 404, 500, 503] {
            let raw = response(&format!("HTTP/1.1 {code} No"), "<root/>");
            assert_eq!(parse_response(&raw), Err(FetchError::Status(code)));
        }
    }

    #[test]
    fn an_html_response_is_refused() {
        let raw = response(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8",
            "<html><body>Login</body></html>",
        );
        assert_eq!(
            parse_response(&raw),
            Err(FetchError::ContentType("text/html".into()))
        );
    }

    #[test]
    fn a_content_type_of_xml_in_any_spelling_is_accepted() {
        for mime in [
            "text/xml",
            "application/xml",
            "APPLICATION/XML",
            "text/xml;charset=utf-8",
            "application/octet-stream",
            "text/plain",
        ] {
            let raw = response(
                &format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}"),
                "<root/>",
            );
            assert!(parse_response(&raw).is_ok(), "{mime} was refused");
        }
    }

    #[test]
    fn a_non_http_reply_is_refused() {
        assert_eq!(
            parse_response(b"hello there\r\n\r\n"),
            Err(FetchError::NotHttp)
        );
        assert_eq!(parse_response(b""), Err(FetchError::NotHttp));
        assert_eq!(
            parse_response(b"HTTP/1.1\r\n\r\n"),
            Err(FetchError::NotHttp)
        );
    }

    #[test]
    fn a_bare_newline_separator_is_tolerated() {
        let raw = b"HTTP/1.0 200 OK\nContent-Type: text/xml\n\n<root/>".to_vec();
        assert_eq!(parse_response(&raw).unwrap(), "<root/>");
    }

    #[test]
    fn an_oversized_body_is_refused() {
        let body = "x".repeat(MAX_BODY_BYTES + 1);
        let raw = response("HTTP/1.1 200 OK\r\nContent-Type: text/xml", &body);
        assert_eq!(parse_response(&raw), Err(FetchError::TooLarge));
    }

    #[test]
    fn oversized_headers_are_refused() {
        let padding = "X-Pad: y\r\n".repeat(MAX_HEADER_BYTES / 8);
        let raw = response(&format!("HTTP/1.1 200 OK\r\n{padding}"), "<root/>");
        assert_eq!(parse_response(&raw), Err(FetchError::TooLarge));
    }

    #[test]
    fn invalid_utf8_in_the_body_is_replaced_not_fatal() {
        let mut raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n<root>".to_vec();
        raw.extend_from_slice(&[0xFF, 0xFE]);
        raw.extend_from_slice(b"</root>");
        let body = parse_response(&raw).unwrap();
        assert!(body.contains('\u{FFFD}'));
        assert!(body.starts_with("<root>"));
    }

    #[test]
    fn the_budgets_stay_inside_the_discovery_window() {
        // The whole discovery pass is a few seconds; a single fetch must not be
        // able to consume it.
        assert!(FETCH_TIMEOUT <= Duration::from_secs(3));
        assert!(CONNECT_TIMEOUT < FETCH_TIMEOUT);
    }

    #[test]
    fn every_failure_explains_itself_in_words() {
        for error in [
            FetchError::Connect,
            FetchError::Timeout,
            FetchError::Io,
            FetchError::Redirect(302),
            FetchError::Status(404),
            FetchError::ContentType("text/html".into()),
            FetchError::TooLarge,
            FetchError::NotHttp,
        ] {
            assert!(!error.reason().is_empty());
        }
    }
}
