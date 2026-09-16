//! Real-world regression fixtures for v1.9 discovery.
//!
//! Each test below is a device seen on an actual network, written down as the
//! evidence a scan would collect from it, with the answer ArcScan is required
//! to give. They exist because the v1.8 classifier gave the wrong answer to
//! several of them in live testing against ArcAtlas — a workstation read as a
//! server, a Canon multifunction suggested as a server, a switch and an access
//! point both read as "network equipment", one NAS counted twice.
//!
//! The governing rule is the one the whole module is built around: **a wrong
//! classification is worse than Unknown.** So several fixtures assert what
//! ArcScan must *not* say, which is the more important half.

#![cfg(test)]

use std::net::Ipv4Addr;

use super::classify::{classify, Classification, ClassifyFacts};
use super::model::{
    Confidence, DeviceType, DiscoveredDevice, DiscoverySource, Evidence, EvidenceKind,
};
use super::reconcile::{reconcile, DeviceCandidate, IdentityClaim, IdentityStrength};
use super::windows::facts::{WindowsFacts, WindowsInterface, WindowsProductType};

/// A device under test, assembled the way a scan would assemble one.
struct Device {
    discovery: DiscoveredDevice,
    ports: Vec<u16>,
    vendor: Option<String>,
    hostname: Option<String>,
    gateway: bool,
    os_guess: Option<String>,
}

impl Device {
    fn new() -> Self {
        Device {
            discovery: DiscoveredDevice::new(Ipv4Addr::new(10, 0, 0, 10)),
            ports: Vec::new(),
            vendor: None,
            hostname: None,
            gateway: false,
            os_guess: None,
        }
    }

    fn ports(mut self, ports: &[u16]) -> Self {
        self.ports = ports.to_vec();
        self
    }

    fn vendor(mut self, vendor: &str) -> Self {
        self.vendor = Some(vendor.into());
        self
    }

    fn hostname(mut self, hostname: &str) -> Self {
        self.hostname = Some(hostname.into());
        self
    }

    fn gateway(mut self) -> Self {
        self.gateway = true;
        self
    }

    fn os_guess(mut self, guess: &str) -> Self {
        self.os_guess = Some(guess.into());
        self
    }

    fn service(mut self, name: &str) -> Self {
        self.discovery.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Service,
            name,
            name,
            Confidence::High,
        ));
        self
    }

    fn model(mut self, model: &str) -> Self {
        self.discovery.add(Evidence::new(
            DiscoverySource::Ssdp,
            EvidenceKind::Model,
            "",
            model,
            Confidence::Medium,
        ));
        self
    }

    /// A manufacturer read off the device itself by a deep probe, as distinct
    /// from the one the OUI table derived from the MAC.
    fn discovered_manufacturer(mut self, source: DiscoverySource, value: &str) -> Self {
        self.discovery.add(Evidence::new(
            source,
            EvidenceKind::Manufacturer,
            "",
            value,
            Confidence::Medium,
        ));
        self
    }

    /// Everything an authenticated Windows query established.
    fn windows(mut self, facts: &WindowsFacts) -> Self {
        for evidence in facts.to_evidence() {
            self.discovery.add(evidence);
        }
        self
    }

    fn classify(&self) -> Classification {
        classify(
            Some(&self.discovery),
            &ClassifyFacts {
                open_ports: &self.ports,
                vendor: self.vendor.as_deref(),
                hostname: self.hostname.as_deref(),
                is_gateway: self.gateway,
                os_guess: self.os_guess.as_deref(),
            },
        )
    }
}

/// A Windows 11 Pro laptop, as WMI reports one.
fn windows_11_workstation() -> WindowsFacts {
    WindowsFacts {
        os_caption: Some("Microsoft Windows 11 Pro".into()),
        os_product: Some("Windows 11".into()),
        os_edition: Some("Pro".into()),
        os_version: Some("10.0.26100".into()),
        os_release: Some("24H2".into()),
        os_build: Some(26100),
        os_architecture: Some("x64".into()),
        product_type: Some(WindowsProductType::Workstation),
        hardware_manufacturer: Some("Dell Inc.".into()),
        hardware_model: Some("Latitude 7450".into()),
        hardware_serial: Some("7SZ1B43".into()),
        system_uuid: Some("4C4C4544-0037-5A10-8051-B4C04F435331".into()),
        computer_name: Some("WS-FINANCE-04".into()),
        domain: Some("corp.example".into()),
        part_of_domain: Some(true),
        workgroup: None,
        interfaces: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. Windows 11 workstation with SMB and RDP
// ---------------------------------------------------------------------------

#[test]
fn a_windows_11_workstation_with_smb_and_rdp_is_never_a_server() {
    // The headline failure from live testing. File sharing and Remote Desktop
    // are what a corporate desktop image switches on; they are not evidence of
    // a server, and no number of them adds up to one.
    let result = Device::new()
        .hostname("WS-FINANCE-04")
        .vendor("Dell Inc")
        .ports(&[135, 139, 445, 3389, 5985])
        .os_guess("Windows")
        .service("_smb._tcp")
        .classify();

    assert_ne!(result.device_type, DeviceType::Server);
    assert_ne!(result.device_type, DeviceType::DomainController);
    // Without credentials the honest answer is the general one: something a
    // person uses or a machine that serves, and ArcScan cannot tell which.
    assert_eq!(result.device_type, DeviceType::Computer);
    assert!(result.confidence.at_least(Confidence::Medium));
}

#[test]
fn the_same_workstation_with_credentials_is_named_exactly() {
    let facts = windows_11_workstation();
    let result = Device::new()
        .hostname("WS-FINANCE-04")
        .vendor("Dell Inc")
        .ports(&[135, 139, 445, 3389])
        .windows(&facts)
        .classify();

    assert_eq!(result.device_type, DeviceType::Workstation);
    assert_eq!(result.confidence, Confidence::High);
    assert!(result
        .evidence
        .iter()
        .any(|line| line.contains("ProductType 1")));
    // And the exact release, which no unauthenticated probe could establish.
    assert_eq!(
        facts.os_summary().as_deref(),
        Some("Windows 11 Pro 24H2 (build 26100, x64)")
    );
}

#[test]
fn the_generic_computer_claim_is_not_listed_as_a_conflict_with_workstation() {
    // "Workstation, and it might also be a Computer" is not a disagreement.
    let result = Device::new()
        .ports(&[445, 3389])
        .windows(&windows_11_workstation())
        .classify();
    assert_eq!(result.device_type, DeviceType::Workstation);
    assert!(!result
        .conflicts
        .iter()
        .any(|claim| claim.device_type == DeviceType::Computer));
}

// ---------------------------------------------------------------------------
// 2. Windows Server, ProductType 3
// ---------------------------------------------------------------------------

#[test]
fn a_windows_server_reports_product_type_three_and_is_named_a_server() {
    let facts = WindowsFacts {
        os_caption: Some("Microsoft Windows Server 2022 Standard".into()),
        os_product: Some("Windows Server 2022".into()),
        os_edition: Some("Standard".into()),
        os_version: Some("10.0.20348".into()),
        os_release: Some("2022".into()),
        os_build: Some(20348),
        os_architecture: Some("x64".into()),
        product_type: Some(WindowsProductType::Server),
        hardware_manufacturer: Some("Dell Inc.".into()),
        hardware_model: Some("PowerEdge R750".into()),
        hardware_serial: Some("J7K2M13".into()),
        system_uuid: Some("4C4C4544-004A-3710-8054-B7C04F324D13".into()),
        computer_name: Some("APP-01".into()),
        domain: Some("corp.example".into()),
        part_of_domain: Some(true),
        ..Default::default()
    };

    let result = Device::new()
        .hostname("APP-01")
        .vendor("Dell Inc")
        .ports(&[135, 445, 3389])
        .windows(&facts)
        .classify();

    assert_eq!(result.device_type, DeviceType::Server);
    assert_eq!(result.confidence, Confidence::High);
    assert!(result
        .evidence
        .iter()
        .any(|line| line.contains("ProductType 3")));
    assert_eq!(
        facts.os_summary().as_deref(),
        Some("Windows Server 2022 Standard 2022 (build 20348, x64)")
    );
}

// ---------------------------------------------------------------------------
// 3. Domain controller, ProductType 2
// ---------------------------------------------------------------------------

#[test]
fn a_domain_controller_reports_product_type_two() {
    let facts = WindowsFacts {
        os_caption: Some("Microsoft Windows Server 2019 Datacenter".into()),
        os_product: Some("Windows Server 2019".into()),
        os_edition: Some("Datacenter".into()),
        os_version: Some("10.0.17763".into()),
        os_build: Some(17763),
        product_type: Some(WindowsProductType::DomainController),
        computer_name: Some("DC-01".into()),
        domain: Some("corp.example".into()),
        part_of_domain: Some(true),
        ..Default::default()
    };

    let result = Device::new()
        .hostname("DC-01")
        .ports(&[53, 88, 135, 389, 445, 636, 3268])
        .windows(&facts)
        .classify();

    assert_eq!(result.device_type, DeviceType::DomainController);
    assert_eq!(result.confidence, Confidence::High);
    assert!(result
        .evidence
        .iter()
        .any(|line| line.contains("ProductType 2")));
    // Server is a generalisation of domain controller, so it is not shown as a
    // competing answer.
    assert!(!result
        .conflicts
        .iter()
        .any(|claim| claim.device_type == DeviceType::Server));
}

#[test]
fn a_directory_service_is_recognised_without_credentials_but_only_at_medium() {
    // Kerberos, LDAP and SMB together are what a directory looks like from
    // outside. Worth saying, never certain: these are open ports.
    let result = Device::new()
        .hostname("DC-02")
        .ports(&[53, 88, 135, 389, 445, 636, 3268])
        .classify();

    assert_eq!(result.device_type, DeviceType::DomainController);
    assert_eq!(result.confidence, Confidence::Medium);
    assert!(result.evidence.iter().any(|line| line.contains("Kerberos")));
}

#[test]
fn smb_and_rdp_without_kerberos_never_reach_domain_controller() {
    let result = Device::new().ports(&[135, 139, 445, 3389]).classify();
    assert_ne!(result.device_type, DeviceType::DomainController);
    assert_ne!(result.device_type, DeviceType::Server);
}

// ---------------------------------------------------------------------------
// 4. Canon network printer
// ---------------------------------------------------------------------------

#[test]
fn a_canon_multifunction_is_a_printer_and_never_a_server() {
    // Reported from live testing: Canon equipment suggested as a server.
    let result = Device::new()
        .hostname("CANON-3F")
        .vendor("Canon Inc.")
        .ports(&[80, 443, 515, 631, 9100])
        .service("_ipp._tcp")
        .service("_printer._tcp")
        .model("imageRUNNER ADVANCE C5535i")
        .classify();

    assert_eq!(result.device_type, DeviceType::Printer);
    assert_eq!(result.confidence, Confidence::High);
    assert_ne!(result.device_type, DeviceType::Server);
}

#[test]
fn a_canon_printer_with_no_mdns_is_still_a_printer() {
    // The harder case: multicast is filtered or switched off, so the only
    // facts are the manufacturer and a raw printing port.
    let result = Device::new()
        .vendor("Canon Inc.")
        .ports(&[80, 443, 9100])
        .classify();

    assert_eq!(result.device_type, DeviceType::Printer);
    assert_eq!(result.confidence, Confidence::Medium);
}

#[test]
fn a_canon_printer_identified_only_by_its_web_server_is_still_a_printer() {
    // Deep scanning reads `Server: Canon HTTP Server` off the embedded web
    // interface, which is the device naming itself.
    let result = Device::new()
        .ports(&[80, 9100])
        .discovered_manufacturer(DiscoverySource::Http, "Canon")
        .classify();

    assert_eq!(result.device_type, DeviceType::Printer);
}

#[test]
fn a_linux_desktop_running_cups_is_not_a_printer() {
    // Port 631 is CUPS, and CUPS runs on desktops. This is the false positive
    // the dedicated-print-port rule exists to avoid.
    let result = Device::new()
        .vendor("Hewlett Packard")
        .ports(&[22, 631])
        .classify();

    assert_ne!(result.device_type, DeviceType::Printer);
}

// ---------------------------------------------------------------------------
// 5. Synology NAS
// ---------------------------------------------------------------------------

#[test]
fn a_synology_nas_is_storage() {
    let result = Device::new()
        .hostname("DiskStation")
        .vendor("Synology Incorporated")
        .ports(&[80, 443, 445, 5000, 5001])
        .service("_smb._tcp")
        .service("_http._tcp")
        .model("DS923+")
        .classify();

    assert_eq!(result.device_type, DeviceType::Nas);
    assert_eq!(result.confidence, Confidence::High);
}

#[test]
fn a_synology_identified_only_by_its_certificate_is_still_storage() {
    let result = Device::new()
        .ports(&[443, 445, 5001])
        .service("_smb._tcp")
        .discovered_manufacturer(DiscoverySource::Tls, "Synology")
        .classify();

    assert_eq!(result.device_type, DeviceType::Nas);
}

// ---------------------------------------------------------------------------
// 6-8. The Ubiquiti line-up: switch, access point, gateway
// ---------------------------------------------------------------------------

#[test]
fn a_ubiquiti_switch_is_a_switch() {
    for model in [
        "USW-24-PoE",
        "USW-Pro-48-PoE",
        "USW Lite 8 PoE",
        "USW-Aggregation",
    ] {
        let result = Device::new()
            .vendor("Ubiquiti Inc")
            .ports(&[22, 80, 443])
            .model(model)
            .classify();
        assert_eq!(result.device_type, DeviceType::Switch, "{model}");
        assert_eq!(result.confidence, Confidence::High, "{model}");
    }
}

#[test]
fn a_ubiquiti_access_point_is_an_access_point() {
    for model in ["U6-Pro", "U6-LR", "U7-Pro", "U7-Pro-Max", "UAP-AC-Pro"] {
        let result = Device::new()
            .vendor("Ubiquiti Inc")
            .ports(&[22, 80, 443])
            .model(model)
            .classify();
        assert_eq!(result.device_type, DeviceType::AccessPoint, "{model}");
        assert_eq!(result.confidence, Confidence::High, "{model}");
    }
}

#[test]
fn a_switch_and_an_access_point_are_never_the_same_answer() {
    // The distinction live testing asked for: before v1.9 both were "network
    // equipment", so nothing downstream could tell them apart.
    let switch = Device::new()
        .vendor("Ubiquiti Inc")
        .model("USW-24-PoE")
        .classify();
    let access_point = Device::new()
        .vendor("Ubiquiti Inc")
        .model("U7-Pro")
        .classify();
    assert_ne!(switch.device_type, access_point.device_type);
}

#[test]
fn a_udm_or_uxg_is_a_gateway() {
    for model in ["UDM-Pro", "UDM-SE", "UXG-Lite", "UXG-Pro", "USG-3P"] {
        let result = Device::new()
            .vendor("Ubiquiti Inc")
            .ports(&[22, 80, 443])
            .model(model)
            .classify();
        assert_eq!(result.device_type, DeviceType::Router, "{model}");
        assert_eq!(result.confidence, Confidence::High, "{model}");
    }
}

#[test]
fn a_udm_that_is_also_the_default_gateway_still_reads_as_a_router() {
    let result = Device::new()
        .vendor("Ubiquiti Inc")
        .model("UDM-Pro")
        .ports(&[53, 80, 443])
        .gateway()
        .classify();
    assert_eq!(result.device_type, DeviceType::Router);
    assert_eq!(result.confidence, Confidence::High);
}

// ---------------------------------------------------------------------------
// 9-10. Dell PowerEdge, and the iDRAC bolted into it
// ---------------------------------------------------------------------------

#[test]
fn a_dell_poweredge_is_server_hardware() {
    let result = Device::new()
        .hostname("ESX-02")
        .vendor("Dell Inc")
        .ports(&[22, 80, 443, 902])
        .model("PowerEdge R750")
        .classify();

    assert_eq!(result.device_type, DeviceType::Server);
    // Medium, not High: the chassis is a server, and only the operating system
    // can confirm that what runs on it is one too.
    assert_eq!(result.confidence, Confidence::Medium);
}

#[test]
fn an_idrac_is_a_management_controller_not_the_server_it_manages() {
    let result = Device::new()
        .hostname("idrac-7SZ1B43")
        .vendor("Dell Inc")
        .ports(&[22, 443, 5900])
        .model("iDRAC9")
        .classify();

    assert_eq!(result.device_type, DeviceType::ManagementController);
    assert_eq!(result.confidence, Confidence::High);
    assert_ne!(result.device_type, DeviceType::Server);
}

#[test]
fn an_ilo_is_a_management_controller() {
    for model in ["Integrated Lights-Out 5", "iLO5", "iLO 6"] {
        let result = Device::new()
            .vendor("Hewlett Packard Enterprise")
            .ports(&[443])
            .model(model)
            .classify();
        assert_eq!(
            result.device_type,
            DeviceType::ManagementController,
            "{model}"
        );
    }
}

#[test]
fn a_poweredge_that_also_exposes_an_idrac_string_reads_as_the_controller() {
    // A BMC's own web interface names the chassis it lives in. When both
    // strings are present the controller wins, because the address being
    // scanned is the controller's.
    let result = Device::new()
        .vendor("Dell Inc")
        .ports(&[443])
        .model("iDRAC9")
        .model("PowerEdge R750")
        .classify();
    assert_eq!(result.device_type, DeviceType::ManagementController);
}

// ---------------------------------------------------------------------------
// 11. Several NICs, one physical Windows machine
// ---------------------------------------------------------------------------

#[test]
fn several_nics_on_one_windows_machine_reconcile_to_one_device() {
    // A server with a wired NIC and a management NIC, found twice by the
    // sweep and reconciled by the system UUID the credentialed query returned.
    let uuid = "4C4C4544-004A-3710-8054-B7C04F324D13";
    let facts = WindowsFacts {
        system_uuid: Some(uuid.into()),
        hardware_serial: Some("J7K2M13".into()),
        hardware_manufacturer: Some("Dell Inc.".into()),
        computer_name: Some("APP-01".into()),
        product_type: Some(WindowsProductType::Server),
        interfaces: vec![
            WindowsInterface {
                mac: Some("AA:BB:CC:00:00:01".into()),
                ipv4: vec!["10.0.0.20".into()],
                description: Some("Intel I350".into()),
            },
            WindowsInterface {
                mac: Some("AA:BB:CC:00:00:02".into()),
                ipv4: vec!["10.0.1.20".into()],
                description: Some("Intel I350 #2".into()),
            },
        ],
        ..Default::default()
    };

    let candidates: Vec<DeviceCandidate> = facts
        .interfaces
        .iter()
        .enumerate()
        .map(|(index, interface)| DeviceCandidate {
            device_id: index as i64 + 1,
            ip: interface.ipv4.first().cloned(),
            mac: interface.mac.clone(),
            hostname: facts.computer_name.clone(),
            identities: vec![
                IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap(),
                IdentityClaim::new(
                    IdentityStrength::HardwareSerial,
                    facts.hardware_manufacturer.as_deref(),
                    facts.hardware_serial.as_deref().unwrap(),
                )
                .unwrap(),
            ],
        })
        .collect();

    let groups = reconcile(&candidates);
    assert_eq!(groups.len(), 1, "one machine, not two");
    let group = &groups[0];
    assert_eq!(group.members, vec![1, 2]);
    // Nothing is lost by the merge: both addresses and both MACs survive.
    assert_eq!(group.addresses, vec!["10.0.0.20", "10.0.1.20"]);
    assert_eq!(group.macs.len(), 2);
    assert!(group.is_multi_homed());
    assert_eq!(group.confidence, Confidence::High);
    assert!(group
        .evidence
        .iter()
        .any(|line| line.contains("system UUID")));
}

#[test]
fn a_nas_on_two_ports_is_counted_once() {
    // The duplicate reported from ArcAtlas. No credentials involved: the SMB
    // server GUID is enough, and it is the same on both interfaces.
    let guid = "4c4c4544-0037-5a10-8051-b4c04f435331";
    let candidates: Vec<DeviceCandidate> = [
        ("10.0.0.30", "00:11:32:aa:bb:01", 1i64),
        ("10.0.0.31", "00:11:32:aa:bb:02", 2),
    ]
    .into_iter()
    .map(|(ip, mac, id)| DeviceCandidate {
        device_id: id,
        ip: Some(ip.into()),
        mac: Some(mac.into()),
        hostname: Some("DiskStation".into()),
        identities: vec![
            IdentityClaim::new(IdentityStrength::VendorUnique, Some("smb"), guid).unwrap(),
        ],
    })
    .collect();

    let groups = reconcile(&candidates);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].addresses.len(), 2);
}

// ---------------------------------------------------------------------------
// 12. Two devices that share a host name and must stay apart
// ---------------------------------------------------------------------------

#[test]
fn two_devices_sharing_a_hostname_are_never_merged() {
    // Duplicate host names are normal: two appliances out of the same box,
    // two machines from one image, two printers a vendor shipped named the
    // same. A merge here would destroy one device's history inside the other's
    // and say nothing on screen about having done it.
    let candidates = vec![
        DeviceCandidate {
            device_id: 1,
            ip: Some("10.0.0.40".into()),
            mac: Some("aa:bb:cc:00:00:11".into()),
            hostname: Some("PRINTER".into()),
            identities: vec![
                IdentityClaim::new(IdentityStrength::Mac, None, "aa:bb:cc:00:00:11").unwrap(),
            ],
        },
        DeviceCandidate {
            device_id: 2,
            ip: Some("10.0.0.41".into()),
            mac: Some("aa:bb:cc:00:00:22".into()),
            hostname: Some("PRINTER".into()),
            identities: vec![
                IdentityClaim::new(IdentityStrength::Mac, None, "aa:bb:cc:00:00:22").unwrap(),
            ],
        },
    ];

    let groups = reconcile(&candidates);
    assert_eq!(groups.len(), 2, "a shared host name is not an identity");
}

#[test]
fn two_machines_from_one_image_sharing_a_placeholder_serial_stay_apart() {
    // A rack of identical hardware all reporting "To Be Filled By O.E.M." must
    // not collapse into one device.
    let candidates: Vec<DeviceCandidate> = (1..=3)
        .map(|id| DeviceCandidate {
            device_id: id,
            ip: Some(format!("10.0.0.5{id}")),
            mac: Some(format!("aa:bb:cc:00:00:{id}{id}")),
            hostname: Some("WIN-GENERIC".into()),
            identities: [
                IdentityClaim::new(
                    IdentityStrength::HardwareSerial,
                    Some("Acme"),
                    "To Be Filled By O.E.M.",
                ),
                IdentityClaim::new(
                    IdentityStrength::Mac,
                    None,
                    &format!("aa:bb:cc:00:00:{id}{id}"),
                ),
            ]
            .into_iter()
            .flatten()
            .collect(),
        })
        .collect();

    assert_eq!(reconcile(&candidates).len(), 3);
}

// ---------------------------------------------------------------------------
// The governing rule
// ---------------------------------------------------------------------------

#[test]
fn a_device_that_says_nothing_stays_unknown() {
    // No evidence, no answer. Preferred over a guess, always.
    let result = Device::new().ports(&[]).classify();
    assert_eq!(result.device_type, DeviceType::Unknown);
    assert_eq!(result.confidence, Confidence::Unknown);
}

#[test]
fn an_open_port_alone_never_reaches_high_confidence() {
    for ports in [vec![22u16], vec![445], vec![3389], vec![80], vec![554]] {
        let result = Device::new().ports(&ports).classify();
        assert!(
            !result.confidence.at_least(Confidence::High),
            "ports {ports:?} reached {:?}",
            result.confidence
        );
    }
}

#[test]
fn no_unauthenticated_fixture_ever_claims_an_exact_windows_release() {
    // The rule that a version claim requires an authenticated answer. Every
    // unauthenticated fixture here is checked for one.
    for device in [
        Device::new().ports(&[445, 3389]).os_guess("Windows"),
        Device::new().ports(&[88, 389, 445]).os_guess("Windows"),
        Device::new()
            .ports(&[445])
            .discovered_manufacturer(DiscoverySource::Http, "Microsoft"),
    ] {
        let has_release = device
            .discovery
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::OsVersion | EvidenceKind::OsBuild));
        assert!(!has_release, "an unauthenticated probe named a release");
    }
}
