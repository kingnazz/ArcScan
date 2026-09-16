//! Manufacturer and model signatures, and what each one is worth.
//!
//! # Why a table
//!
//! Deep scanning collects strings devices say about themselves: an HTTP
//! `Server` header, a TLS certificate subject, a login page title, an SSH
//! banner. Each is a sentence written by the manufacturer, and a handful of
//! them name the product exactly. `Server: Canon HTTP Server` is not an
//! inference about a Canon device; it is a Canon device saying so.
//!
//! The table is the whole of that knowledge, in one place, so that adding a
//! product means adding a row rather than editing a rule — and so that a
//! reviewer can read what ArcScan is prepared to believe in one screen.
//!
//! # What a match is worth
//!
//! A signature yields a *claim*, never a verdict. The strongest rows say both
//! who made the thing and what it is (`iDRAC` is a Dell management controller
//! and cannot be anything else), and those carry [`Confidence::High`]. Rows
//! that name a maker but not a product (`Apache`) carry no device type at all.
//! Everything in between is Medium: strong enough to shape an answer,
//! never enough to be one on its own.
//!
//! Nothing here is allowed to name a Windows *version*. `Microsoft-IIS/10.0`
//! says Windows, and the "10.0" is the IIS version, not the operating system's
//! — reading it as Windows 10 would be exactly the kind of confident wrong
//! answer v1.9 exists to remove.

use crate::discovery::model::{Confidence, DeviceType};

/// What a matched signature establishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The manufacturer, when the string names one.
    pub manufacturer: Option<&'static str>,
    /// The product family, when the string names one.
    pub model: Option<&'static str>,
    /// What the product *is*, when the string settles it.
    pub device_type: Option<DeviceType>,
    /// The operating-system family, when the string implies one. Never a
    /// version: a web server's version is not its host's.
    pub os_family: Option<&'static str>,
    pub confidence: Confidence,
    /// How the evidence line reads in the drawer.
    pub label: &'static str,
}

const fn sig(
    manufacturer: Option<&'static str>,
    model: Option<&'static str>,
    device_type: Option<DeviceType>,
    os_family: Option<&'static str>,
    confidence: Confidence,
    label: &'static str,
) -> Signature {
    Signature {
        manufacturer,
        model,
        device_type,
        os_family,
        confidence,
        label,
    }
}

/// Needles matched, lowercased, as substrings of a device-supplied string.
///
/// Order matters: the first match wins, so the most specific rows come first.
/// `idrac` must be tested before `dell`, or a management controller would be
/// recorded as the server it is bolted into.
static SIGNATURES: &[(&str, Signature)] = &[
    // ---- Management controllers -------------------------------------
    //
    // First in the table on purpose. A BMC has its own address, its own MAC and
    // its own credentials, and every one of them also names the server vendor.
    // Matching the vendor first is how an iDRAC becomes a PowerEdge in the
    // inventory, which is the confusion this ordering exists to prevent.
    (
        "idrac",
        sig(
            Some("Dell"),
            Some("iDRAC"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "Dell iDRAC management controller",
        ),
    ),
    (
        "integrated lights-out",
        sig(
            Some("HPE"),
            Some("iLO"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE Integrated Lights-Out management controller",
        ),
    ),
    (
        "hp-ilo",
        sig(
            Some("HPE"),
            Some("iLO"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE iLO management controller",
        ),
    ),
    (
        "hpe-ilo",
        sig(
            Some("HPE"),
            Some("iLO"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE iLO management controller",
        ),
    ),
    (
        "ilo 4",
        sig(
            Some("HPE"),
            Some("iLO 4"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE iLO 4 management controller",
        ),
    ),
    (
        "ilo 5",
        sig(
            Some("HPE"),
            Some("iLO 5"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE iLO 5 management controller",
        ),
    ),
    (
        "ilo 6",
        sig(
            Some("HPE"),
            Some("iLO 6"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "HPE iLO 6 management controller",
        ),
    ),
    (
        "xclarity",
        sig(
            Some("Lenovo"),
            Some("XClarity Controller"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "Lenovo XClarity management controller",
        ),
    ),
    (
        "cimc",
        sig(
            Some("Cisco"),
            Some("CIMC"),
            Some(DeviceType::ManagementController),
            None,
            Confidence::High,
            "Cisco CIMC management controller",
        ),
    ),
    (
        "supermicro",
        sig(
            Some("Supermicro"),
            None,
            None,
            None,
            Confidence::Medium,
            "Supermicro management interface",
        ),
    ),
    // ---- Server hardware --------------------------------------------
    (
        "poweredge",
        sig(
            Some("Dell"),
            Some("PowerEdge"),
            Some(DeviceType::Server),
            None,
            Confidence::Medium,
            "Dell PowerEdge server hardware",
        ),
    ),
    (
        "proliant",
        sig(
            Some("HPE"),
            Some("ProLiant"),
            Some(DeviceType::Server),
            None,
            Confidence::Medium,
            "HPE ProLiant server hardware",
        ),
    ),
    // ---- Storage -----------------------------------------------------
    (
        "diskstation",
        sig(
            Some("Synology"),
            Some("DiskStation"),
            Some(DeviceType::Nas),
            None,
            Confidence::High,
            "Synology DiskStation",
        ),
    ),
    (
        "rackstation",
        sig(
            Some("Synology"),
            Some("RackStation"),
            Some(DeviceType::Nas),
            None,
            Confidence::High,
            "Synology RackStation",
        ),
    ),
    (
        "synology",
        sig(
            Some("Synology"),
            None,
            Some(DeviceType::Nas),
            None,
            Confidence::Medium,
            "Synology storage appliance",
        ),
    ),
    (
        "qnap",
        sig(
            Some("QNAP"),
            None,
            Some(DeviceType::Nas),
            None,
            Confidence::Medium,
            "QNAP storage appliance",
        ),
    ),
    (
        "truenas",
        sig(
            Some("iXsystems"),
            Some("TrueNAS"),
            Some(DeviceType::Nas),
            None,
            Confidence::High,
            "TrueNAS storage appliance",
        ),
    ),
    (
        "asustor",
        sig(
            Some("ASUSTOR"),
            None,
            Some(DeviceType::Nas),
            None,
            Confidence::Medium,
            "ASUSTOR storage appliance",
        ),
    ),
    // ---- Printers ----------------------------------------------------
    //
    // A printer's embedded web server names its maker and, often, its model.
    // These are High because a "Canon HTTP Server" is not something a
    // non-printer answers with.
    (
        "canon http server",
        sig(
            Some("Canon"),
            None,
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Canon printer web server",
        ),
    ),
    (
        "hp http server",
        sig(
            Some("HP"),
            None,
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "HP printer web server",
        ),
    ),
    (
        "brother",
        sig(
            Some("Brother"),
            None,
            Some(DeviceType::Printer),
            None,
            Confidence::Medium,
            "Brother printer web interface",
        ),
    ),
    (
        "kyocera",
        sig(
            Some("Kyocera"),
            None,
            Some(DeviceType::Printer),
            None,
            Confidence::Medium,
            "Kyocera printer web interface",
        ),
    ),
    (
        "lexmark",
        sig(
            Some("Lexmark"),
            None,
            Some(DeviceType::Printer),
            None,
            Confidence::Medium,
            "Lexmark printer web interface",
        ),
    ),
    (
        "imagerunner",
        sig(
            Some("Canon"),
            Some("imageRUNNER"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Canon imageRUNNER",
        ),
    ),
    (
        "imageclass",
        sig(
            Some("Canon"),
            Some("imageCLASS"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Canon imageCLASS",
        ),
    ),
    (
        "laserjet",
        sig(
            Some("HP"),
            Some("LaserJet"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "HP LaserJet",
        ),
    ),
    (
        "officejet",
        sig(
            Some("HP"),
            Some("OfficeJet"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "HP OfficeJet",
        ),
    ),
    (
        "workcentre",
        sig(
            Some("Xerox"),
            Some("WorkCentre"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Xerox WorkCentre",
        ),
    ),
    (
        "versalink",
        sig(
            Some("Xerox"),
            Some("VersaLink"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Xerox VersaLink",
        ),
    ),
    (
        "ecosys",
        sig(
            Some("Kyocera"),
            Some("ECOSYS"),
            Some(DeviceType::Printer),
            None,
            Confidence::High,
            "Kyocera ECOSYS",
        ),
    ),
    // ---- Network equipment -------------------------------------------
    (
        "udm",
        sig(
            Some("Ubiquiti"),
            Some("UniFi Dream Machine"),
            Some(DeviceType::Router),
            None,
            Confidence::High,
            "UniFi Dream Machine gateway",
        ),
    ),
    (
        "dream machine",
        sig(
            Some("Ubiquiti"),
            Some("UniFi Dream Machine"),
            Some(DeviceType::Router),
            None,
            Confidence::High,
            "UniFi Dream Machine gateway",
        ),
    ),
    (
        "uxg",
        sig(
            Some("Ubiquiti"),
            Some("UniFi Next-Gen Gateway"),
            Some(DeviceType::Router),
            None,
            Confidence::High,
            "UniFi Next-Gen Gateway",
        ),
    ),
    (
        "pfsense",
        sig(
            Some("Netgate"),
            Some("pfSense"),
            Some(DeviceType::Firewall),
            None,
            Confidence::High,
            "pfSense firewall",
        ),
    ),
    (
        "opnsense",
        sig(
            Some("Deciso"),
            Some("OPNsense"),
            Some(DeviceType::Firewall),
            None,
            Confidence::High,
            "OPNsense firewall",
        ),
    ),
    (
        "fortigate",
        sig(
            Some("Fortinet"),
            Some("FortiGate"),
            Some(DeviceType::Firewall),
            None,
            Confidence::High,
            "FortiGate firewall",
        ),
    ),
    (
        "sonicwall",
        sig(
            Some("SonicWall"),
            None,
            Some(DeviceType::Firewall),
            None,
            Confidence::High,
            "SonicWall firewall",
        ),
    ),
    (
        "pan-os",
        sig(
            Some("Palo Alto Networks"),
            Some("PAN-OS"),
            Some(DeviceType::Firewall),
            None,
            Confidence::High,
            "Palo Alto PAN-OS firewall",
        ),
    ),
    (
        "mikrotik",
        sig(
            Some("MikroTik"),
            Some("RouterOS"),
            None,
            Some("network_os"),
            Confidence::Medium,
            "MikroTik RouterOS",
        ),
    ),
    (
        "routeros",
        sig(
            Some("MikroTik"),
            Some("RouterOS"),
            None,
            Some("network_os"),
            Confidence::Medium,
            "MikroTik RouterOS",
        ),
    ),
    // ---- Cameras ------------------------------------------------------
    (
        "hikvision",
        sig(
            Some("Hikvision"),
            None,
            Some(DeviceType::Camera),
            None,
            Confidence::Medium,
            "Hikvision video device",
        ),
    ),
    (
        "dahua",
        sig(
            Some("Dahua"),
            None,
            Some(DeviceType::Camera),
            None,
            Confidence::Medium,
            "Dahua video device",
        ),
    ),
    (
        "axis",
        sig(
            Some("Axis Communications"),
            None,
            Some(DeviceType::Camera),
            None,
            Confidence::Medium,
            "Axis video device",
        ),
    ),
    // ---- Operating systems, from their web servers --------------------
    //
    // `Microsoft-IIS/10.0` establishes Windows and nothing more. The 10.0 is
    // the IIS version; reading it as the OS version is the mistake this row
    // exists to not make.
    (
        "microsoft-iis",
        sig(
            Some("Microsoft"),
            None,
            None,
            Some("windows"),
            Confidence::Medium,
            "Microsoft IIS (the host runs Windows; the version shown is IIS's, not Windows's)",
        ),
    ),
    (
        "microsoft-httpapi",
        sig(
            Some("Microsoft"),
            None,
            None,
            Some("windows"),
            Confidence::Medium,
            "Windows HTTP stack (the host runs Windows)",
        ),
    ),
];

/// The first signature whose needle appears in `haystack`.
///
/// Case-insensitive, first-match-wins. The caller supplies whatever string the
/// device offered; this does not care which protocol it came from, which is
/// what lets one table serve HTTP headers, TLS subjects and banners alike.
///
/// # How a needle matches
///
/// A needle containing a separator (`canon http server`, `microsoft-iis`,
/// `ilo 5`) is matched as a substring, because the separator already makes it
/// specific enough to be safe.
///
/// A bare word (`udm`, `axis`, `cimc`) is matched against whole tokens, plus a
/// trailing generation number: `idrac` matches the token `idrac9`, and `ilo`
/// matches `ilo5`, because vendors write the model that way as often as not.
///
/// It does *not* match a token that merely contains the needle. Short vendor
/// abbreviations are the useful ones and also the dangerous ones: a substring
/// `udm` matches "cloudmesh" and a substring `axis` matches "praxis", and a
/// scanner that decides a build server is a camera because its host name
/// contains four incidental letters is worse than one that decides nothing.
pub fn match_signature(haystack: &str) -> Option<&'static Signature> {
    let lower = haystack.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    SIGNATURES
        .iter()
        .find(|(needle, _)| {
            if needle.contains(|c: char| !c.is_ascii_alphanumeric()) {
                lower.contains(needle)
            } else {
                tokens.iter().any(|token| matches_token(token, needle))
            }
        })
        .map(|(_, signature)| signature)
}

/// True when `token` is `needle`, or `needle` followed only by digits.
fn matches_token(token: &str, needle: &str) -> bool {
    match token.strip_prefix(needle) {
        None => false,
        Some("") => true,
        Some(rest) => rest.chars().all(|c| c.is_ascii_digit()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_management_controller_is_matched_before_its_server_vendor() {
        // The ordering rule that keeps an iDRAC from being recorded as the
        // PowerEdge it is bolted into.
        let found = match_signature("Server: iDRAC/9").unwrap();
        assert_eq!(found.device_type, Some(DeviceType::ManagementController));
        assert_eq!(found.manufacturer, Some("Dell"));
    }

    #[test]
    fn an_ilo_login_page_is_a_management_controller_not_a_server() {
        let found = match_signature("<title>iLO 5</title>").unwrap();
        assert_eq!(found.device_type, Some(DeviceType::ManagementController));
        assert_eq!(found.manufacturer, Some("HPE"));
    }

    #[test]
    fn a_generation_number_attached_to_the_model_still_matches() {
        // Vendors write "iDRAC9" as often as "iDRAC 9", and an authentication
        // realm is usually the terse spelling.
        for spelling in ["iDRAC9", "iDRAC 9", "iDRAC/9", "idrac8"] {
            assert_eq!(
                match_signature(spelling).unwrap().device_type,
                Some(DeviceType::ManagementController),
                "{spelling} should identify a management controller"
            );
        }
    }

    #[test]
    fn poweredge_on_its_own_is_server_hardware() {
        let found = match_signature("Dell PowerEdge R740").unwrap();
        assert_eq!(found.device_type, Some(DeviceType::Server));
        assert_eq!(found.model, Some("PowerEdge"));
    }

    #[test]
    fn a_canon_web_server_is_a_printer() {
        let found = match_signature("Canon HTTP Server").unwrap();
        assert_eq!(found.device_type, Some(DeviceType::Printer));
        assert_eq!(found.manufacturer, Some("Canon"));
        assert_eq!(found.confidence, Confidence::High);
    }

    #[test]
    fn a_synology_appliance_is_storage() {
        let found = match_signature("Synology DiskStation DS923+").unwrap();
        assert_eq!(found.device_type, Some(DeviceType::Nas));
        assert_eq!(found.model, Some("DiskStation"));
    }

    #[test]
    fn a_firewall_names_itself() {
        assert_eq!(
            match_signature("pfSense").unwrap().device_type,
            Some(DeviceType::Firewall)
        );
        assert_eq!(
            match_signature("FortiGate-60F").unwrap().device_type,
            Some(DeviceType::Firewall)
        );
    }

    #[test]
    fn iis_establishes_windows_and_refuses_to_name_a_version() {
        let found = match_signature("Microsoft-IIS/10.0").unwrap();
        assert_eq!(found.os_family, Some("windows"));
        // No OS version, and no device type: a Windows host running IIS could
        // be a workstation or a server, and only ProductType settles that.
        assert_eq!(found.device_type, None);
        assert!(found.label.contains("not Windows"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(match_signature("SYNOLOGY").is_some());
        assert!(match_signature("synology").is_some());
        assert!(match_signature("SyNoLoGy").is_some());
    }

    #[test]
    fn an_ordinary_web_server_matches_nothing() {
        assert!(match_signature("Apache/2.4.58 (Debian)").is_none());
        assert!(match_signature("nginx/1.24.0").is_none());
        assert!(match_signature("").is_none());
    }

    #[test]
    fn a_short_needle_does_not_match_inside_a_longer_word() {
        // `udm` is a real UniFi gateway prefix and four incidental letters in
        // plenty of other strings. A substring match would call both a router.
        assert!(match_signature("cloudmesh-worker-01").is_none());
        assert!(match_signature("praxis-billing").is_none());
        assert!(match_signature("decimcast").is_none());
        // The real thing still matches, separators and all.
        assert_eq!(
            match_signature("UDM-Pro").unwrap().device_type,
            Some(DeviceType::Router)
        );
        assert_eq!(
            match_signature("UDM SE").unwrap().device_type,
            Some(DeviceType::Router)
        );
        assert_eq!(
            match_signature("AXIS P3245-LVE").unwrap().device_type,
            Some(DeviceType::Camera)
        );
    }

    #[test]
    fn hyphenated_needles_still_match_as_substrings() {
        assert!(match_signature("Microsoft-IIS/10.0").is_some());
        assert!(match_signature("HP-iLO-Server/1.30").is_some());
        assert!(match_signature("PAN-OS 11.1").is_some());
    }

    #[test]
    fn no_signature_ever_claims_an_os_version() {
        // The single rule that keeps a web server's version out of the OS
        // column. Enforced structurally: there is no field to put one in.
        for (_, signature) in SIGNATURES {
            if let Some(family) = signature.os_family {
                assert!(
                    !family.chars().any(|c| c.is_ascii_digit()),
                    "{family} looks like a version, not a family"
                );
            }
        }
    }
}
