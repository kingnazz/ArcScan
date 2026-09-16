//! Turning a CIM/WMI answer into [`WindowsFacts`].
//!
//! Split out from the process launcher on purpose, and compiled on every
//! platform even though the launcher is Windows-only. The launcher is a few
//! lines of "start a child, write to stdin, read stdout"; everything that can
//! actually be *wrong* — a placeholder serial treated as an identity, an
//! edition parsed out of the wrong half of a caption, an architecture string
//! that differs between locales — is here, where fixtures can pin it down on a
//! Linux CI runner.

use serde_json::Value;

use super::facts::{WindowsFacts, WindowsInterface, WindowsProductType};
use super::release;

/// SMBIOS values that are present, well-formed, and mean nothing.
///
/// Motherboard vendors ship these, and a fleet of machines from one supplier
/// will share them exactly. Treating one as an identity is how two unrelated
/// computers become one device, so they are refused here — at the parser, not
/// at the consumer, so that no later caller can forget.
const PLACEHOLDER_IDENTIFIERS: &[&str] = &[
    "to be filled by o.e.m.",
    "to be filled by oem",
    "system serial number",
    "default string",
    "not specified",
    "not available",
    "not applicable",
    "none",
    "null",
    "n/a",
    "na",
    "unknown",
    "0",
    "123456789",
    "0123456789",
    "invalid",
    "chassis serial number",
    "base board serial number",
    "filled by oem",
    "oem",
    "xxxxxxx",
];

/// True when a serial, tag or UUID is a placeholder rather than an identity.
pub fn is_placeholder_identifier(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if PLACEHOLDER_IDENTIFIERS.contains(&lower.as_str()) {
        return true;
    }
    // All-zero and all-F UUIDs, and any run of a single repeated character,
    // are what a machine reports when it has nothing to report.
    let significant: String = lower
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if significant.is_empty() {
        return true;
    }
    if significant.len() > 1
        && significant
            .chars()
            .all(|c| c == significant.as_bytes()[0] as char)
    {
        return true;
    }
    false
}

/// Clean a scalar WMI string: trim, drop placeholders, refuse the empty.
fn clean(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Clean a value that is going to be used as an identity.
fn clean_identifier(value: Option<&Value>) -> Option<String> {
    let text = clean(value)?;
    if is_placeholder_identifier(&text) {
        return None;
    }
    Some(text)
}

fn clean_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(b) => Some(*b),
        // PowerShell's ConvertTo-Json emits real booleans, but a WMI bridge
        // that stringifies everything is not worth failing over.
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_i64().map(|v| v != 0),
        _ => None,
    }
}

fn clean_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Normalize `OSArchitecture`, which is a localized display string.
///
/// Windows reports `64-bit` in English, `64 bits` in French and `64 位` in
/// Chinese. Matching on the digits rather than the words is what keeps a
/// German-language server from exporting a blank architecture column.
pub fn normalize_architecture(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.contains("arm") {
        return Some(if lower.contains("64") { "arm64" } else { "arm" }.to_string());
    }
    if lower.contains("64") {
        return Some("x64".to_string());
    }
    if lower.contains("32") || lower.contains("x86") {
        return Some("x86".to_string());
    }
    None
}

/// Split `Microsoft Windows 11 Pro` into the product and the edition.
///
/// Returns `(product, edition)`. The edition is `None` when the caption names
/// only a product, which is what an evaluation image or a Server Core install
/// sometimes reports.
pub fn split_caption(caption: &str) -> (Option<String>, Option<String>) {
    let text = caption.trim();
    if text.is_empty() {
        return (None, None);
    }
    // The vendor prefix is noise in every caption that has it.
    let text = text
        .strip_prefix("Microsoft ")
        .or_else(|| text.strip_prefix("Microsoft® "))
        .unwrap_or(text)
        .trim();

    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() || !tokens[0].eq_ignore_ascii_case("windows") {
        // Not a caption this parser understands. Report the whole thing as the
        // product rather than inventing a split.
        return (Some(text.to_string()), None);
    }

    let mut taken = 1usize;
    if tokens.len() > 1 && tokens[1].eq_ignore_ascii_case("server") {
        taken = 2;
        // `Windows Server 2022`, and `Windows Server 2012 R2`.
        if let Some(year) = tokens.get(2) {
            if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
                taken = 3;
                if tokens.get(3).is_some_and(|r| r.eq_ignore_ascii_case("r2")) {
                    taken = 4;
                }
            }
        }
    } else if let Some(version) = tokens.get(1) {
        // `Windows 11`, `Windows 10`, `Windows 8.1`, `Windows 7`, and the
        // named releases that predate them.
        let is_version = version.chars().all(|c| c.is_ascii_digit() || c == '.')
            && version.chars().any(|c| c.is_ascii_digit());
        let is_named = ["vista", "xp", "2000"]
            .iter()
            .any(|n| version.eq_ignore_ascii_case(n));
        if is_version || is_named {
            taken = 2;
        }
    }

    let product = tokens[..taken].join(" ");
    let edition = tokens[taken..].join(" ");
    let edition = edition.trim();
    (
        Some(product),
        (!edition.is_empty()).then(|| edition.to_string()),
    )
}

/// Read the network adapters out of the report.
///
/// A machine with a wired NIC, a wireless NIC and a hypervisor bridge is one
/// computer that ArcScan would otherwise find three times; this is what lets
/// [`crate::discovery::reconcile`] put it back together.
fn parse_interfaces(value: Option<&Value>) -> Vec<WindowsInterface> {
    let Some(Value::Array(nics)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for nic in nics {
        let mac = clean(nic.get("MACAddress"))
            .as_deref()
            .and_then(crate::scanner::normalize_mac);
        let ipv4 = match nic.get("IPAddress") {
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|v| clean(Some(v)))
                // IPv6 is recorded elsewhere; this list is what ArcScan scans
                // and what reconciliation matches on.
                .filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
                .collect(),
            Some(one @ Value::String(_)) => clean(Some(one))
                .filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        let description = clean(nic.get("Description"));
        if mac.is_none() && ipv4.is_empty() {
            continue;
        }
        out.push(WindowsInterface {
            mac,
            ipv4,
            description,
        });
    }
    out
}

/// Parse the JSON document the collection script writes to stdout.
///
/// Unknown keys are ignored and missing sections are absent rather than fatal:
/// an account with rights to `Win32_OperatingSystem` but not to
/// `Win32_ComputerSystemProduct` should still produce the OS half rather than
/// nothing at all.
pub fn parse_report(json: &str) -> Result<WindowsFacts, String> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| format!("the machine's reply was not valid JSON: {e}"))?;

    // The script reports its own failures in a field of their own, so a refusal
    // by the remote machine reads as a refusal rather than as an empty answer.
    if let Some(error) = root.get("error").and_then(Value::as_str) {
        let error = error.trim();
        if !error.is_empty() {
            return Err(error.to_string());
        }
    }

    let section = |name: &str| root.get(name).filter(|v| !v.is_null());
    fn field<'a>(sec: Option<&'a Value>, key: &str) -> Option<&'a Value> {
        sec.and_then(|s| s.get(key))
    }

    let os = section("os");
    let cs = section("computer_system");
    let product = section("product");
    let bios = section("bios");

    // ---- Operating system ---------------------------------------------
    let os_caption = clean(field(os, "Caption"));
    let (os_product, os_edition) = match os_caption.as_deref() {
        Some(caption) => split_caption(caption),
        None => (None, None),
    };
    let os_version = clean(field(os, "Version"));
    let product_type = clean_i64(field(os, "ProductType")).and_then(WindowsProductType::from_code);

    // The build comes from `BuildNumber` when it is there and from the version
    // string when it is not; both are reported by every supported release, and
    // taking either keeps a partial answer useful.
    let os_build = clean(field(os, "BuildNumber"))
        .and_then(|b| b.trim().parse::<u32>().ok())
        .or_else(|| os_version.as_deref().and_then(release::build_from_version));

    // The feature-update label is derived, never reported. It needs the product
    // type, because build 26100 is Windows 11 24H2 on a client and Windows
    // Server 2025 on a server.
    let found = os_build.and_then(|build| release::lookup(build, product_type));
    let os_release = found.as_ref().map(|r| r.release.clone());
    // The caption is authoritative for the product and is left alone; the
    // table only fills a gap.
    let os_product = os_product.or_else(|| found.map(|r| r.product));

    // ---- Membership ----------------------------------------------------
    let part_of_domain = clean_bool(field(cs, "PartOfDomain"));
    let domain = clean(field(cs, "Domain"));
    let workgroup = clean(field(cs, "Workgroup")).or_else(|| {
        // A machine that is not domain-joined reports its workgroup in the
        // Domain field, and leaves Workgroup null.
        (part_of_domain == Some(false))
            .then(|| domain.clone())
            .flatten()
    });

    let facts = WindowsFacts {
        os_caption,
        os_product,
        os_edition,
        os_version,
        os_release,
        os_build,
        os_architecture: clean(field(os, "OSArchitecture"))
            .as_deref()
            .and_then(normalize_architecture),
        product_type,

        // ---- Hardware --------------------------------------------------
        computer_name: clean(field(cs, "Name")),
        hardware_manufacturer: clean(field(cs, "Manufacturer"))
            .or_else(|| clean(field(product, "Vendor"))),
        hardware_model: clean(field(cs, "Model")).or_else(|| clean(field(product, "Name"))),
        // The service tag first: `IdentifyingNumber` is what Dell, HP and
        // Lenovo put a support contract against. The BIOS serial is the same
        // string on most hardware and a fallback on the rest.
        hardware_serial: clean_identifier(field(product, "IdentifyingNumber"))
            .or_else(|| clean_identifier(field(bios, "SerialNumber"))),
        system_uuid: clean_identifier(field(product, "UUID")).map(|u| u.to_uppercase()),

        part_of_domain,
        domain,
        workgroup,

        // ---- Interfaces ------------------------------------------------
        interfaces: parse_interfaces(root.get("nics")),
    };

    if facts.is_empty() {
        return Err("the machine answered, but reported nothing ArcScan could use".into());
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_caption_splits_into_product_and_edition() {
        assert_eq!(
            split_caption("Microsoft Windows 11 Pro"),
            (Some("Windows 11".into()), Some("Pro".into()))
        );
        assert_eq!(
            split_caption("Microsoft Windows 10 Enterprise"),
            (Some("Windows 10".into()), Some("Enterprise".into()))
        );
    }

    #[test]
    fn a_server_caption_keeps_the_year_with_the_product() {
        assert_eq!(
            split_caption("Microsoft Windows Server 2022 Standard"),
            (Some("Windows Server 2022".into()), Some("Standard".into()))
        );
        assert_eq!(
            split_caption("Microsoft Windows Server 2019 Datacenter Evaluation"),
            (
                Some("Windows Server 2019".into()),
                Some("Datacenter Evaluation".into())
            )
        );
        assert_eq!(
            split_caption("Microsoft Windows Server 2012 R2 Standard"),
            (
                Some("Windows Server 2012 R2".into()),
                Some("Standard".into())
            )
        );
    }

    #[test]
    fn a_caption_with_no_edition_reports_none_rather_than_empty() {
        assert_eq!(
            split_caption("Microsoft Windows Server 2025"),
            (Some("Windows Server 2025".into()), None)
        );
    }

    #[test]
    fn an_unrecognised_caption_is_kept_whole_rather_than_split_wrongly() {
        let (product, edition) = split_caption("Some Other Operating System 4.2");
        assert_eq!(product.as_deref(), Some("Some Other Operating System 4.2"));
        assert_eq!(edition, None);
    }

    #[test]
    fn architecture_is_read_from_the_digits_not_the_words() {
        assert_eq!(normalize_architecture("64-bit").as_deref(), Some("x64"));
        assert_eq!(normalize_architecture("32-bit").as_deref(), Some("x86"));
        assert_eq!(
            normalize_architecture("ARM 64-bit").as_deref(),
            Some("arm64")
        );
        // Localized strings still resolve, which is the point of matching on
        // the digits.
        assert_eq!(normalize_architecture("64 bits").as_deref(), Some("x64"));
        assert_eq!(normalize_architecture("").as_deref(), None);
    }

    #[test]
    fn oem_placeholder_serials_are_refused_as_identities() {
        for placeholder in [
            "To Be Filled By O.E.M.",
            "System Serial Number",
            "Default string",
            "None",
            "0",
            "  ",
            "00000000-0000-0000-0000-000000000000",
            "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
            "Not Specified",
        ] {
            assert!(
                is_placeholder_identifier(placeholder),
                "{placeholder} should not be usable as an identity"
            );
        }
    }

    #[test]
    fn a_real_service_tag_is_not_mistaken_for_a_placeholder() {
        for real in [
            "7SZ1B43",
            "4C4C4544-0037-5A10-8051-B4C04F435331",
            "CZC1234ABC",
            "MXL0123456",
        ] {
            assert!(
                !is_placeholder_identifier(real),
                "{real} is a real identity"
            );
        }
    }

    #[test]
    fn malformed_json_is_an_error_not_an_empty_answer() {
        assert!(parse_report("not json").is_err());
    }

    #[test]
    fn a_reported_error_is_surfaced_rather_than_read_as_no_facts() {
        let json = r#"{"error": "Access is denied. (0x80070005)"}"#;
        let err = parse_report(json).unwrap_err();
        assert!(err.contains("Access is denied"));
    }

    #[test]
    fn an_answer_with_nothing_usable_in_it_is_an_error() {
        assert!(parse_report(r#"{"os": {}}"#).is_err());
        assert!(parse_report(r#"{}"#).is_err());
    }
}
