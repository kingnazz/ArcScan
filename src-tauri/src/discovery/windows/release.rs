//! Turning a Windows build number into the release people actually say.
//!
//! `Win32_OperatingSystem` reports `10.0.26100`, and nobody asks for the
//! machine running 26100 — they ask for the one on 24H2. The mapping is a
//! published fact per build, so it is a table rather than a rule, and it is
//! keyed on *both* the build and whether the machine is a client or a server:
//! build 26100 is Windows 11 24H2 on a workstation and Windows Server 2025 on a
//! server, which is precisely why `ProductType` has to be known before this is
//! consulted.
//!
//! Nothing here guesses. A build that is not in the table returns `None`, and
//! the caller reports the build number alone rather than inventing a label.

use super::facts::WindowsProductType;

/// Client (workstation) builds, newest first.
///
/// `(build, product, release)`. The product is carried because the NT version
/// is `10.0` for both Windows 10 and Windows 11, so the build is the only thing
/// that separates them.
const CLIENT_BUILDS: &[(u32, &str, &str)] = &[
    (26200, "Windows 11", "25H2"),
    (26100, "Windows 11", "24H2"),
    (22631, "Windows 11", "23H2"),
    (22621, "Windows 11", "22H2"),
    (22000, "Windows 11", "21H2"),
    (19045, "Windows 10", "22H2"),
    (19044, "Windows 10", "21H2"),
    (19043, "Windows 10", "21H1"),
    (19042, "Windows 10", "20H2"),
    (19041, "Windows 10", "2004"),
    (18363, "Windows 10", "1909"),
    (18362, "Windows 10", "1903"),
    (17763, "Windows 10", "1809"),
    (17134, "Windows 10", "1803"),
    (16299, "Windows 10", "1709"),
    (15063, "Windows 10", "1703"),
    (14393, "Windows 10", "1607"),
    (10586, "Windows 10", "1511"),
    (10240, "Windows 10", "1507"),
    (9600, "Windows 8.1", "8.1"),
    (7601, "Windows 7", "SP1"),
];

/// Server builds, newest first. Server releases are named by year, and the
/// Azure-edition half-year releases carry their own label.
const SERVER_BUILDS: &[(u32, &str, &str)] = &[
    (26100, "Windows Server 2025", "2025"),
    (25398, "Windows Server", "23H2"),
    (20348, "Windows Server 2022", "2022"),
    (17763, "Windows Server 2019", "2019"),
    (14393, "Windows Server 2016", "2016"),
    (9600, "Windows Server 2012 R2", "2012 R2"),
    (9200, "Windows Server 2012", "2012"),
    (7601, "Windows Server 2008 R2", "SP1"),
];

/// What a build number says about the release, for a known product type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The marketed product, e.g. `Windows 11` or `Windows Server 2022`.
    pub product: String,
    /// The feature update or release label, e.g. `24H2`.
    pub release: String,
}

/// Look up a build.
///
/// `product_type` decides which table is consulted. `None` means ArcScan does
/// not know whether this is a client or a server, and the answer is `None`
/// rather than a coin toss: a shared build number would otherwise be reported
/// as the wrong product half the time.
pub fn lookup(build: u32, product_type: Option<WindowsProductType>) -> Option<Release> {
    let table = match product_type? {
        WindowsProductType::Workstation => CLIENT_BUILDS,
        WindowsProductType::Server | WindowsProductType::DomainController => SERVER_BUILDS,
    };
    table
        .iter()
        .find(|(known, _, _)| *known == build)
        .map(|(_, product, release)| Release {
            product: (*product).to_string(),
            release: (*release).to_string(),
        })
}

/// Parse the build out of a WMI version string such as `10.0.26100` or
/// `10.0.26100.2314`, or from a bare build number.
pub fn build_from_version(version: &str) -> Option<u32> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('.').collect();
    let candidate = match parts.len() {
        // A bare `26100`.
        1 => parts[0],
        // `10.0.26100` and `10.0.26100.2314` both put the build third.
        _ if parts.len() >= 3 => parts[2],
        // `10.0` says nothing about the build.
        _ => return None,
    };
    candidate.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_build_resolves_to_a_feature_update() {
        let found = lookup(26100, Some(WindowsProductType::Workstation)).unwrap();
        assert_eq!(found.product, "Windows 11");
        assert_eq!(found.release, "24H2");
    }

    #[test]
    fn the_same_build_is_a_different_product_on_a_server() {
        // 26100 is Windows 11 24H2 and Windows Server 2025. Reading it without
        // the product type would be a coin toss, which is the whole reason
        // ProductType is collected first.
        let client = lookup(26100, Some(WindowsProductType::Workstation)).unwrap();
        let server = lookup(26100, Some(WindowsProductType::Server)).unwrap();
        assert_eq!(client.product, "Windows 11");
        assert_eq!(server.product, "Windows Server 2025");
    }

    #[test]
    fn a_domain_controller_reads_from_the_server_table() {
        let found = lookup(20348, Some(WindowsProductType::DomainController)).unwrap();
        assert_eq!(found.product, "Windows Server 2022");
    }

    #[test]
    fn windows_10_builds_resolve_to_their_own_releases() {
        assert_eq!(
            lookup(19045, Some(WindowsProductType::Workstation))
                .unwrap()
                .release,
            "22H2"
        );
        assert_eq!(
            lookup(19045, Some(WindowsProductType::Workstation))
                .unwrap()
                .product,
            "Windows 10"
        );
    }

    #[test]
    fn an_unknown_build_gets_no_label_rather_than_a_guess() {
        assert!(lookup(99999, Some(WindowsProductType::Workstation)).is_none());
    }

    #[test]
    fn without_a_product_type_no_release_is_claimed() {
        assert!(lookup(26100, None).is_none());
    }

    #[test]
    fn builds_parse_out_of_every_shape_wmi_reports() {
        assert_eq!(build_from_version("10.0.26100"), Some(26100));
        assert_eq!(build_from_version("10.0.26100.2314"), Some(26100));
        assert_eq!(build_from_version("26100"), Some(26100));
        assert_eq!(build_from_version(" 10.0.20348 "), Some(20348));
    }

    #[test]
    fn a_version_with_no_build_in_it_parses_to_nothing() {
        assert_eq!(build_from_version("10.0"), None);
        assert_eq!(build_from_version(""), None);
        assert_eq!(build_from_version("not-a-version"), None);
        assert_eq!(build_from_version("10.0.notanumber"), None);
    }
}
