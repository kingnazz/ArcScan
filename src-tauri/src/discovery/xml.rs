//! A deliberately small XML reader for UPnP device descriptions.
//!
//! # Threat model
//!
//! The document comes from an unauthenticated device on the local network, over
//! plain HTTP, and ArcScan asked for it because that device said to. It is
//! hostile input in every sense, and the classic XML attacks all apply:
//! external entities that read local files, entity expansion that turns a few
//! kilobytes into gigabytes of memory, and nesting deep enough to blow a
//! recursive parser's stack.
//!
//! # What this parser will not do
//!
//! * **No DTD, at all.** A document containing `<!DOCTYPE` is refused before a
//!   single field is read. UPnP descriptions have no legitimate use for one,
//!   and refusing outright removes external entities, parameter entities and
//!   entity expansion in a single rule rather than trying to allow a safe
//!   subset.
//! * **No entity resolution.** Only the five predefined entities and numeric
//!   character references are decoded, each into exactly one character. There is
//!   no entity table to poison, so no expansion bomb exists.
//! * **No network access**, no file access, no schema fetching.
//! * **No recursion.** Nesting is tracked with an explicit stack and capped at
//!   [`MAX_DEPTH`], so a deeply nested document is refused rather than
//!   exhausting the stack.
//!
//! Every value it returns is a plain string, bounded in length, with control
//! characters stripped — which is what makes it safe for the interface to render
//! as text. Markup inside a field arrives as literal characters (`&lt;script&gt;`
//! decodes to `<script>` *as text*), never as anything the interface interprets.
//!
//! Icons are counted and never fetched; presentation URLs are recorded and never
//! opened.

use super::model::{sanitize_field, MAX_FIELD_CHARS};

/// Largest description document accepted, in bytes. Real ones are 1–4 KB.
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;

/// Deepest element nesting accepted.
pub const MAX_DEPTH: usize = 32;

/// Most characters of text accumulated for one element before the rest is
/// discarded. Comfortably longer than any field ArcScan keeps.
pub const MAX_TEXT_CHARS: usize = 1_024;

/// Most services and embedded devices recorded from one document.
pub const MAX_SERVICES: usize = 32;
pub const MAX_EMBEDDED_DEVICES: usize = 16;

/// Most elements processed in one document, as a final backstop against a
/// document that is small but pathologically fragmented.
pub const MAX_ELEMENTS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlError {
    TooLarge,
    /// A `<!DOCTYPE` was present. Refused rather than parsed.
    Doctype,
    TooDeep,
    TooManyElements,
    /// Nothing that looks like a UPnP device description.
    NotADescription,
}

impl XmlError {
    pub fn reason(&self) -> &'static str {
        match self {
            XmlError::TooLarge => "the description document is larger than ArcScan will read",
            XmlError::Doctype => "the description document declares a DTD",
            XmlError::TooDeep => "the description document is nested too deeply",
            XmlError::TooManyElements => "the description document has too many elements",
            XmlError::NotADescription => "the document is not a UPnP device description",
        }
    }
}

/// The fields ArcScan takes from a device description. Everything else in the
/// document is walked past.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Description {
    pub device_type: Option<String>,
    pub friendly_name: Option<String>,
    pub manufacturer: Option<String>,
    pub manufacturer_url: Option<String>,
    pub model_description: Option<String>,
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub model_url: Option<String>,
    pub serial_number: Option<String>,
    pub udn: Option<String>,
    pub presentation_url: Option<String>,
    /// Service type URNs advertised by the root device and its children.
    pub services: Vec<String>,
    /// Device types of embedded devices, which often say more than the root's.
    pub embedded_types: Vec<String>,
    /// How many icons were declared. Recorded so the drawer can say the device
    /// offers one; never fetched.
    pub icon_count: usize,
}

impl Description {
    pub fn is_empty(&self) -> bool {
        self.device_type.is_none()
            && self.friendly_name.is_none()
            && self.manufacturer.is_none()
            && self.model_name.is_none()
            && self.udn.is_none()
            && self.services.is_empty()
    }
}

/// Parse a device description.
pub fn parse(document: &str) -> Result<Description, XmlError> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(XmlError::TooLarge);
    }
    // Refused before anything else is looked at. Checked case-insensitively so
    // `<!doctype` cannot slip past, and on the raw text so it cannot be hidden
    // behind an entity — there is no entity table to hide behind.
    if contains_ignore_case(document, "<!doctype") {
        return Err(XmlError::Doctype);
    }

    let chars: Vec<char> = document.chars().collect();
    let mut pos = 0usize;
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut out = Description::default();
    let mut elements = 0usize;
    /// How many `<device>` elements are open. 1 is the root device.
    let mut device_depth = 0usize;
    let mut saw_root = false;

    while pos < chars.len() {
        if chars[pos] != '<' {
            if text.chars().count() < MAX_TEXT_CHARS {
                text.push(chars[pos]);
            }
            pos += 1;
            continue;
        }

        // A `<` that starts a construct. Everything below advances `pos` past it.
        if starts_with(&chars, pos, "<!--") {
            pos = find(&chars, pos + 4, "-->").map_or(chars.len(), |i| i + 3);
            continue;
        }
        if starts_with(&chars, pos, "<![CDATA[") {
            let end = find(&chars, pos + 9, "]]>").unwrap_or(chars.len());
            for c in &chars[pos + 9..end.min(chars.len())] {
                if text.chars().count() >= MAX_TEXT_CHARS {
                    break;
                }
                text.push(*c);
            }
            pos = (end + 3).min(chars.len());
            continue;
        }
        if starts_with(&chars, pos, "<?") {
            pos = find(&chars, pos + 2, "?>").map_or(chars.len(), |i| i + 2);
            continue;
        }
        if starts_with(&chars, pos, "<!") {
            // Any other declaration. The only one that matters is DOCTYPE, and
            // that was refused above.
            pos = find(&chars, pos + 2, ">").map_or(chars.len(), |i| i + 1);
            continue;
        }

        let Some(close) = find(&chars, pos + 1, ">") else {
            break; // an unterminated tag ends the document
        };
        let raw: String = chars[pos + 1..close].iter().collect();
        pos = close + 1;

        elements += 1;
        if elements > MAX_ELEMENTS {
            return Err(XmlError::TooManyElements);
        }

        if let Some(name) = raw.strip_prefix('/') {
            let name = local_name(name.trim());
            let value = decode(&text);
            text.clear();
            // Tolerate a mismatched closing tag by unwinding to it when it is
            // on the stack, and ignoring it when it is not. A device with a
            // sloppy serialiser should still yield its name.
            if let Some(index) = stack.iter().rposition(|open| *open == name) {
                let popped_device = stack[index..].iter().filter(|n| *n == "device").count();
                stack.truncate(index);
                if index == stack.len() {
                    assign(&mut out, &stack, &name, &value, device_depth);
                }
                device_depth = device_depth.saturating_sub(popped_device);
            }
            continue;
        }

        let self_closing = raw.trim_end().ends_with('/');
        let body = raw.trim_end().trim_end_matches('/');
        let name = local_name(body.split_whitespace().next().unwrap_or("").trim());
        if name.is_empty() {
            continue;
        }
        if name == "root" {
            saw_root = true;
        }
        text.clear();

        if self_closing {
            if name == "icon" {
                out.icon_count = out.icon_count.saturating_add(1);
            }
            continue;
        }

        if stack.len() >= MAX_DEPTH {
            return Err(XmlError::TooDeep);
        }
        if name == "device" {
            device_depth += 1;
        }
        if name == "icon" {
            out.icon_count = out.icon_count.saturating_add(1);
        }
        stack.push(name);
    }

    if !saw_root && out.is_empty() {
        return Err(XmlError::NotADescription);
    }
    Ok(out)
}

/// Record one closed element's text against the description.
///
/// `device_depth` of 1 means the element sits inside the *root* device;
/// anything deeper belongs to an embedded device, whose fields must not
/// overwrite the root's. Service types are collected from every depth, because
/// a television's useful service list often hangs off an embedded device.
fn assign(
    out: &mut Description,
    stack: &[String],
    name: &str,
    value: &str,
    device_depth: usize,
) {
    let parent = stack.last().map(String::as_str).unwrap_or("");
    let clean = sanitize_field(value);

    if name == "servicetype" && parent == "service" {
        if let Some(v) = clean {
            if out.services.len() < MAX_SERVICES && !out.services.contains(&v) {
                out.services.push(v);
            }
        }
        return;
    }

    if parent != "device" {
        return;
    }

    // An embedded device contributes only its type, never a name or a model
    // that would shadow the root device's own.
    if device_depth > 1 {
        if name == "devicetype" {
            if let Some(v) = clean {
                if out.embedded_types.len() < MAX_EMBEDDED_DEVICES && !out.embedded_types.contains(&v)
                {
                    out.embedded_types.push(v);
                }
            }
        }
        return;
    }

    // First value wins for every root field: a document that repeats
    // `<friendlyName>` twice is a quirk, and taking the first keeps two reads of
    // the same device identical.
    let slot = match name {
        "devicetype" => &mut out.device_type,
        "friendlyname" => &mut out.friendly_name,
        "manufacturer" => &mut out.manufacturer,
        "manufacturerurl" => &mut out.manufacturer_url,
        "modeldescription" => &mut out.model_description,
        "modelname" => &mut out.model_name,
        "modelnumber" => &mut out.model_number,
        "modelurl" => &mut out.model_url,
        "serialnumber" => &mut out.serial_number,
        "udn" => &mut out.udn,
        "presentationurl" => &mut out.presentation_url,
        _ => return,
    };
    if slot.is_none() {
        *slot = clean;
    }
}

/// Strip a namespace prefix and lowercase, so `upnp:friendlyName`,
/// `friendlyName` and `FRIENDLYNAME` are all the same element.
fn local_name(raw: &str) -> String {
    let bare = raw.rsplit(':').next().unwrap_or(raw);
    bare.to_ascii_lowercase()
}

/// Decode the five predefined entities and numeric character references.
///
/// Nothing else is decoded, because nothing else is declared: with DTDs refused
/// there is no entity table, so `&anything;` is left exactly as written rather
/// than resolved. Each reference produces at most one character, which is what
/// makes an expansion bomb impossible by construction.
fn decode(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // A reference is short by definition; anything longer is literal text.
        let limit = (i + 12).min(chars.len());
        let Some(semi) = (i + 1..limit).find(|j| chars[*j] == ';') else {
            out.push('&');
            i += 1;
            continue;
        };
        let entity: String = chars[i + 1..semi].iter().collect();
        let decoded = match entity.as_str() {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            other => numeric_reference(other),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                i = semi + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// Decode `#38` and `#x26` into a character, refusing anything that is not a
/// valid scalar value.
fn numeric_reference(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    let value = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(value)
}

fn starts_with(chars: &[char], pos: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(i, c)| chars.get(pos + i) == Some(&c))
}

fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    if from >= chars.len() || n.is_empty() {
        return None;
    }
    (from..=chars.len().saturating_sub(n.len())).find(|i| chars[*i..*i + n.len()] == n[..])
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle_lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
    <friendlyName>Acme Hub 6</friendlyName>
    <manufacturer>Acme Networks</manufacturer>
    <manufacturerURL>http://192.0.2.1/</manufacturerURL>
    <modelDescription>Residential gateway</modelDescription>
    <modelName>Hub 6</modelName>
    <modelNumber>AH6-2000</modelNumber>
    <modelURL>http://192.0.2.1/model</modelURL>
    <serialNumber>SN-0000-TEST</serialNumber>
    <UDN>uuid:11111111-2222-3333-4444-555555555555</UDN>
    <presentationURL>http://192.0.2.1/</presentationURL>
    <iconList>
      <icon><mimetype>image/png</mimetype><url>/icon.png</url></icon>
    </iconList>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
        <controlURL>/ctl</controlURL>
      </service>
    </serviceList>
    <deviceList>
      <device>
        <deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
        <friendlyName>WAN Device</friendlyName>
        <serviceList>
          <service>
            <serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType>
          </service>
        </serviceList>
      </device>
    </deviceList>
  </device>
</root>"#;

    #[test]
    fn reads_every_field_from_a_real_gateway_description() {
        let d = parse(REAL).unwrap();
        assert_eq!(
            d.device_type.as_deref(),
            Some("urn:schemas-upnp-org:device:InternetGatewayDevice:1")
        );
        assert_eq!(d.friendly_name.as_deref(), Some("Acme Hub 6"));
        assert_eq!(d.manufacturer.as_deref(), Some("Acme Networks"));
        assert_eq!(d.manufacturer_url.as_deref(), Some("http://192.0.2.1/"));
        assert_eq!(d.model_description.as_deref(), Some("Residential gateway"));
        assert_eq!(d.model_name.as_deref(), Some("Hub 6"));
        assert_eq!(d.model_number.as_deref(), Some("AH6-2000"));
        assert_eq!(d.model_url.as_deref(), Some("http://192.0.2.1/model"));
        assert_eq!(d.serial_number.as_deref(), Some("SN-0000-TEST"));
        assert_eq!(
            d.udn.as_deref(),
            Some("uuid:11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(d.presentation_url.as_deref(), Some("http://192.0.2.1/"));
        assert_eq!(d.icon_count, 1);
        assert_eq!(d.services.len(), 2);
        assert_eq!(
            d.embedded_types,
            vec!["urn:schemas-upnp-org:device:WANDevice:1"]
        );
    }

    #[test]
    fn an_embedded_device_never_overwrites_the_root_name() {
        let d = parse(REAL).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Acme Hub 6"));
    }

    #[test]
    fn a_doctype_is_refused_before_anything_is_read() {
        let doc = r#"<?xml version="1.0"?>
<!DOCTYPE root [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<root><device><friendlyName>&xxe;</friendlyName></device></root>"#;
        assert_eq!(parse(doc), Err(XmlError::Doctype));
        // Case and whitespace cannot smuggle one past.
        assert_eq!(
            parse("<!doctype root><root><device/></root>"),
            Err(XmlError::Doctype)
        );
        assert_eq!(
            parse("<root/><!DocType x>"),
            Err(XmlError::Doctype)
        );
    }

    #[test]
    fn an_entity_expansion_bomb_cannot_be_built() {
        // The billion-laughs shape, without the DOCTYPE that would have been
        // refused outright. With no entity table, `&lol9;` is literal text.
        let doc = "<root><device><friendlyName>&lol9;&lol9;&lol9;</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("&lol9;&lol9;&lol9;"));
    }

    #[test]
    fn only_the_predefined_and_numeric_entities_decode() {
        let doc = "<root><device><friendlyName>a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos; &#65; &#x42; &unknown;</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(
            d.friendly_name.as_deref(),
            Some("a & b <c> \"d\" 'e' A B &unknown;")
        );
    }

    #[test]
    fn markup_inside_a_field_stays_text() {
        let doc = "<root><device><friendlyName>&lt;script&gt;alert(1)&lt;/script&gt;</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        // Decoded to characters, which the interface renders as text. It is a
        // string, not an element, and nothing downstream treats it as markup.
        assert_eq!(d.friendly_name.as_deref(), Some("<script>alert(1)</script>"));
    }

    #[test]
    fn a_real_script_element_is_walked_past_not_executed_or_captured() {
        let doc = "<root><device><script>alert(1)</script><friendlyName>Safe</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Safe"));
    }

    #[test]
    fn cdata_is_read_as_text() {
        let doc = "<root><device><friendlyName><![CDATA[Living <Room> & Co]]></friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Living <Room> & Co"));
    }

    #[test]
    fn an_unterminated_cdata_block_ends_the_document_cleanly() {
        let doc = "<root><device><friendlyName><![CDATA[never closed";
        let d = parse(doc).unwrap();
        assert!(d.friendly_name.is_none());
    }

    #[test]
    fn namespaced_and_oddly_cased_element_names_are_recognised() {
        let doc = r#"<upnp:root xmlns:upnp="urn:x"><UPNP:Device><upnp:FriendlyName>Prefixed</upnp:FriendlyName><Upnp:ModelName>M1</Upnp:ModelName></UPNP:Device></upnp:root>"#;
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Prefixed"));
        assert_eq!(d.model_name.as_deref(), Some("M1"));
    }

    #[test]
    fn comments_and_processing_instructions_are_skipped() {
        let doc = "<?xml version=\"1.0\"?><!-- <friendlyName>Ignored</friendlyName> --><root><device><friendlyName>Real</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Real"));
    }

    #[test]
    fn a_duplicated_field_keeps_the_first_value() {
        let doc = "<root><device><friendlyName>First</friendlyName><friendlyName>Second</friendlyName></device></root>";
        assert_eq!(parse(doc).unwrap().friendly_name.as_deref(), Some("First"));
    }

    #[test]
    fn an_oversized_document_is_refused() {
        let doc = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        assert_eq!(parse(&doc), Err(XmlError::TooLarge));
    }

    #[test]
    fn an_oversized_field_is_capped_rather_than_dropped() {
        let long = "N".repeat(MAX_TEXT_CHARS * 4);
        let doc = format!("<root><device><friendlyName>{long}</friendlyName></device></root>");
        let name = parse(&doc).unwrap().friendly_name.unwrap();
        assert_eq!(name.chars().count(), MAX_FIELD_CHARS);
    }

    #[test]
    fn deep_nesting_is_refused_rather_than_recursed_into() {
        let doc = format!(
            "<root>{}{}</root>",
            "<a>".repeat(MAX_DEPTH + 10),
            "</a>".repeat(MAX_DEPTH + 10)
        );
        assert_eq!(parse(&doc), Err(XmlError::TooDeep));
    }

    #[test]
    fn a_document_with_too_many_elements_is_refused() {
        let doc = format!("<root>{}</root>", "<a/>".repeat(MAX_ELEMENTS + 10));
        assert_eq!(parse(&doc), Err(XmlError::TooManyElements));
    }

    #[test]
    fn service_and_embedded_device_lists_are_bounded() {
        let mut doc = String::from("<root><device><serviceList>");
        for i in 0..(MAX_SERVICES + 40) {
            doc.push_str(&format!(
                "<service><serviceType>urn:x:service:S{i}:1</serviceType></service>"
            ));
        }
        doc.push_str("</serviceList><deviceList>");
        for i in 0..(MAX_EMBEDDED_DEVICES + 20) {
            doc.push_str(&format!(
                "<device><deviceType>urn:x:device:D{i}:1</deviceType></device>"
            ));
        }
        doc.push_str("</deviceList></device></root>");
        let d = parse(&doc).unwrap();
        assert_eq!(d.services.len(), MAX_SERVICES);
        assert_eq!(d.embedded_types.len(), MAX_EMBEDDED_DEVICES);
    }

    #[test]
    fn a_repeated_service_type_is_recorded_once() {
        let doc = "<root><device><serviceList><service><serviceType>urn:x:service:S:1</serviceType></service><service><serviceType>urn:x:service:S:1</serviceType></service></serviceList></device></root>";
        assert_eq!(parse(doc).unwrap().services.len(), 1);
    }

    #[test]
    fn control_characters_are_stripped_from_every_value() {
        let doc = "<root><device><friendlyName>Bad\u{0}\u{7}Name\u{1b}</friendlyName></device></root>";
        let name = parse(doc).unwrap().friendly_name.unwrap();
        assert_eq!(name, "Bad Name");
        assert!(!name.chars().any(char::is_control));
    }

    #[test]
    fn a_document_that_is_not_a_description_is_refused() {
        assert_eq!(parse(""), Err(XmlError::NotADescription));
        assert_eq!(parse("not xml at all"), Err(XmlError::NotADescription));
        assert_eq!(
            parse("<html><body>Login</body></html>"),
            Err(XmlError::NotADescription)
        );
    }

    #[test]
    fn a_truncated_document_yields_what_it_managed_to_say() {
        let doc = "<root><device><friendlyName>Half</friendlyName><modelName>Cut";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Half"));
        assert_eq!(d.model_name, None);
    }

    #[test]
    fn mismatched_closing_tags_do_not_derail_the_parse() {
        let doc = "<root><device><friendlyName>Name</wrong></friendlyName><modelName>M</modelName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.model_name.as_deref(), Some("M"));
    }

    #[test]
    fn self_closing_elements_do_not_unbalance_the_stack() {
        let doc = "<root><device><icon/><br/><friendlyName>Balanced</friendlyName></device></root>";
        let d = parse(doc).unwrap();
        assert_eq!(d.friendly_name.as_deref(), Some("Balanced"));
        assert_eq!(d.icon_count, 1);
    }

    #[test]
    fn attributes_never_become_values() {
        let doc = r#"<root><device friendlyName="FromAttribute"><friendlyName>FromElement</friendlyName></device></root>"#;
        assert_eq!(
            parse(doc).unwrap().friendly_name.as_deref(),
            Some("FromElement")
        );
    }

    #[test]
    fn parsing_never_panics_on_hostile_or_random_input() {
        let fragments = [
            "<", ">", "</", "<!", "<!-", "<![", "<?", "&#;", "&#x;", "&#xFFFFFFFF;", "]]>",
            "<device>", "</device>", "<root", "\u{0}", "é", "<a b=\"", "&#55296;",
        ];
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..3_000 {
            let mut doc = String::new();
            for _ in 0..(next() % 40) {
                doc.push_str(fragments[(next() as usize) % fragments.len()]);
            }
            let _ = parse(&doc);
        }
    }
}
