//! What a credentialed Windows query establishes, and what it is worth.
//!
//! Every field here came from the machine answering a question about itself
//! over an authenticated channel. That is a different kind of fact from
//! anything else ArcScan collects: it is not an inference from a port, a TTL or
//! a manufacturer prefix, so it is the only evidence allowed to name an exact
//! Windows edition — and the only evidence allowed to settle workstation versus
//! server.

use crate::discovery::model::{Confidence, DeviceType, DiscoverySource, Evidence, EvidenceKind};

/// `Win32_OperatingSystem.ProductType`.
///
/// Three values, defined by Microsoft, reported by the operating system about
/// itself. This is the fact the v1.9 work exists to collect: it outranks every
/// guess built on SMB, RDP or an open port, because those describe what a
/// machine is willing to do and this describes what it *is*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsProductType {
    /// `1` — a client OS. A workstation, whatever services it exposes.
    Workstation,
    /// `2` — a server holding a domain controller role.
    DomainController,
    /// `3` — a server that is not a domain controller.
    Server,
}

impl WindowsProductType {
    /// Map the raw WMI integer. Anything outside 1..=3 is refused rather than
    /// bent into the nearest neighbour.
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(WindowsProductType::Workstation),
            2 => Some(WindowsProductType::DomainController),
            3 => Some(WindowsProductType::Server),
            _ => None,
        }
    }

    pub fn code(self) -> i64 {
        match self {
            WindowsProductType::Workstation => 1,
            WindowsProductType::DomainController => 2,
            WindowsProductType::Server => 3,
        }
    }

    /// The device type this product type establishes.
    pub fn device_type(self) -> DeviceType {
        match self {
            WindowsProductType::Workstation => DeviceType::Workstation,
            WindowsProductType::DomainController => DeviceType::DomainController,
            WindowsProductType::Server => DeviceType::Server,
        }
    }

    /// How the evidence line reads in the drawer.
    pub fn label(self) -> &'static str {
        match self {
            WindowsProductType::Workstation => "Windows ProductType 1 (workstation)",
            WindowsProductType::DomainController => "Windows ProductType 2 (domain controller)",
            WindowsProductType::Server => "Windows ProductType 3 (server)",
        }
    }
}

/// One network interface as Windows reports it.
///
/// Collected because a machine with a wired NIC, a wireless NIC and a
/// hypervisor bridge is one computer that ArcScan would otherwise find three
/// times. See [`crate::discovery::reconcile`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsInterface {
    pub mac: Option<String>,
    pub ipv4: Vec<String>,
    pub description: Option<String>,
}

/// Everything one credentialed query established.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsFacts {
    // ---- Operating system ---------------------------------------------
    /// The raw `Caption`, e.g. `Microsoft Windows 11 Pro`. Kept verbatim so a
    /// technician can see exactly what the machine said.
    pub os_caption: Option<String>,
    /// The product alone, e.g. `Windows 11`.
    pub os_product: Option<String>,
    /// The edition alone, e.g. `Pro`, `Datacenter`.
    pub os_edition: Option<String>,
    /// The NT version, e.g. `10.0.26100`.
    pub os_version: Option<String>,
    /// The feature update, e.g. `24H2`. Derived from the build, never reported
    /// directly by WMI.
    pub os_release: Option<String>,
    pub os_build: Option<u32>,
    /// `x64`, `arm64`, `x86`.
    pub os_architecture: Option<String>,
    pub product_type: Option<WindowsProductType>,

    // ---- Hardware -----------------------------------------------------
    pub hardware_manufacturer: Option<String>,
    pub hardware_model: Option<String>,
    /// The service tag, express code or chassis serial.
    pub hardware_serial: Option<String>,
    /// The SMBIOS UUID. The strongest identity a machine can offer.
    pub system_uuid: Option<String>,

    // ---- Membership ---------------------------------------------------
    pub computer_name: Option<String>,
    pub domain: Option<String>,
    pub part_of_domain: Option<bool>,
    pub workgroup: Option<String>,

    // ---- Interfaces ---------------------------------------------------
    pub interfaces: Vec<WindowsInterface>,
}

impl WindowsFacts {
    /// True when nothing at all was established, so a caller can refuse to
    /// record an empty credentialed result as a success.
    pub fn is_empty(&self) -> bool {
        self.os_caption.is_none()
            && self.os_version.is_none()
            && self.product_type.is_none()
            && self.hardware_manufacturer.is_none()
            && self.hardware_model.is_none()
            && self.system_uuid.is_none()
            && self.computer_name.is_none()
            && self.interfaces.is_empty()
    }

    /// The device type this answer establishes, if any.
    ///
    /// Only `ProductType` decides. Nothing else in a credentialed answer is
    /// allowed to: a model name saying "PowerEdge" is a strong hint, but the
    /// operating system saying `ProductType 1` on a PowerEdge means somebody is
    /// running Windows 11 on server hardware, and the workstation answer is the
    /// true one.
    pub fn device_type(&self) -> Option<DeviceType> {
        self.product_type.map(WindowsProductType::device_type)
    }

    /// A one-line summary for the drawer, e.g.
    /// `Windows 11 Pro 24H2 (build 26100, x64)`.
    pub fn os_summary(&self) -> Option<String> {
        let product = self.os_product.as_deref()?;
        let mut out = product.to_string();
        if let Some(edition) = self.os_edition.as_deref() {
            out.push(' ');
            out.push_str(edition);
        }
        if let Some(release) = self.os_release.as_deref() {
            out.push(' ');
            out.push_str(release);
        }
        let mut parenthetical = Vec::new();
        if let Some(build) = self.os_build {
            parenthetical.push(format!("build {build}"));
        }
        if let Some(arch) = self.os_architecture.as_deref() {
            parenthetical.push(arch.to_string());
        }
        if !parenthetical.is_empty() {
            out.push_str(&format!(" ({})", parenthetical.join(", ")));
        }
        Some(out)
    }

    /// Everything here as discovery evidence, at High confidence throughout.
    ///
    /// High is correct and not generous: the machine was asked directly, with
    /// credentials it accepted, and it answered about itself. There is no
    /// stronger evidence available to a scanner short of the operator typing
    /// the answer in.
    pub fn to_evidence(&self) -> Vec<Evidence> {
        let mut out = Vec::new();
        let mut push = |kind: EvidenceKind, key: &str, value: &str| {
            let value = value.trim();
            if value.is_empty() {
                return;
            }
            out.push(Evidence::new(
                DiscoverySource::WindowsCredentialed,
                kind,
                key,
                value,
                Confidence::High,
            ));
        };

        push(EvidenceKind::OsFamily, "", "Windows");
        if let Some(v) = &self.os_product {
            push(EvidenceKind::OsProduct, "", v);
        }
        if let Some(v) = &self.os_edition {
            push(EvidenceKind::OsEdition, "", v);
        }
        if let Some(v) = &self.os_release {
            push(EvidenceKind::OsVersion, "release", v);
        }
        if let Some(v) = &self.os_version {
            push(EvidenceKind::OsVersion, "nt", v);
        }
        if let Some(v) = self.os_build {
            push(EvidenceKind::OsBuild, "", &v.to_string());
        }
        if let Some(v) = &self.os_architecture {
            push(EvidenceKind::OsArchitecture, "", v);
        }
        if let Some(v) = self.product_type {
            push(EvidenceKind::WindowsProductType, "", &v.code().to_string());
        }
        if let Some(v) = &self.hardware_manufacturer {
            push(EvidenceKind::Manufacturer, "", v);
        }
        if let Some(v) = &self.hardware_model {
            push(EvidenceKind::Model, "", v);
        }
        if let Some(v) = &self.hardware_serial {
            push(EvidenceKind::SerialNumber, "", v);
        }
        if let Some(v) = &self.system_uuid {
            push(EvidenceKind::SystemUuid, "", v);
        }
        if let Some(v) = &self.computer_name {
            push(EvidenceKind::Hostname, "", v);
        }
        // The domain a machine is joined to, or the workgroup it is not. Only
        // one of the two is meaningful, decided by `PartOfDomain`, because
        // Windows reports the string `WORKGROUP` in the Domain field of a
        // machine that is not joined to anything.
        match (self.part_of_domain, &self.domain, &self.workgroup) {
            (Some(true), Some(domain), _) => push(EvidenceKind::DomainMembership, "domain", domain),
            (_, _, Some(workgroup)) => {
                push(EvidenceKind::DomainMembership, "workgroup", workgroup)
            }
            (None, Some(domain), None) => push(EvidenceKind::DomainMembership, "domain", domain),
            _ => {}
        }
        for interface in &self.interfaces {
            if let Some(mac) = &interface.mac {
                push(EvidenceKind::ProtocolIdentifier, "windows_nic_mac", mac);
            }
            for ip in &interface.ipv4 {
                push(EvidenceKind::Ipv4Address, "windows_nic", ip);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_type_codes_map_to_the_three_documented_values() {
        assert_eq!(
            WindowsProductType::from_code(1),
            Some(WindowsProductType::Workstation)
        );
        assert_eq!(
            WindowsProductType::from_code(2),
            Some(WindowsProductType::DomainController)
        );
        assert_eq!(
            WindowsProductType::from_code(3),
            Some(WindowsProductType::Server)
        );
    }

    #[test]
    fn an_undocumented_product_type_is_refused() {
        assert_eq!(WindowsProductType::from_code(0), None);
        assert_eq!(WindowsProductType::from_code(4), None);
        assert_eq!(WindowsProductType::from_code(-1), None);
    }

    #[test]
    fn each_product_type_names_its_device_type() {
        assert_eq!(
            WindowsProductType::Workstation.device_type(),
            DeviceType::Workstation
        );
        assert_eq!(
            WindowsProductType::DomainController.device_type(),
            DeviceType::DomainController
        );
        assert_eq!(WindowsProductType::Server.device_type(), DeviceType::Server);
    }

    #[test]
    fn the_os_summary_reads_as_a_technician_would_say_it() {
        let facts = WindowsFacts {
            os_product: Some("Windows 11".into()),
            os_edition: Some("Pro".into()),
            os_release: Some("24H2".into()),
            os_build: Some(26100),
            os_architecture: Some("x64".into()),
            ..Default::default()
        };
        assert_eq!(
            facts.os_summary().as_deref(),
            Some("Windows 11 Pro 24H2 (build 26100, x64)")
        );
    }

    #[test]
    fn a_summary_needs_a_product_and_degrades_gracefully_without_the_rest() {
        assert_eq!(WindowsFacts::default().os_summary(), None);
        let facts = WindowsFacts {
            os_product: Some("Windows Server 2022".into()),
            ..Default::default()
        };
        assert_eq!(facts.os_summary().as_deref(), Some("Windows Server 2022"));
    }

    #[test]
    fn product_type_alone_decides_the_device_type() {
        // Server hardware running a client OS is a workstation. The model name
        // does not get a vote.
        let facts = WindowsFacts {
            hardware_model: Some("PowerEdge R740".into()),
            product_type: Some(WindowsProductType::Workstation),
            ..Default::default()
        };
        assert_eq!(facts.device_type(), Some(DeviceType::Workstation));
    }

    #[test]
    fn without_a_product_type_no_device_type_is_claimed() {
        let facts = WindowsFacts {
            os_product: Some("Windows 11".into()),
            hardware_model: Some("OptiPlex 7090".into()),
            ..Default::default()
        };
        assert_eq!(facts.device_type(), None);
    }

    #[test]
    fn evidence_carries_the_product_type_as_its_own_fact() {
        let facts = WindowsFacts {
            product_type: Some(WindowsProductType::Server),
            ..Default::default()
        };
        let evidence = facts.to_evidence();
        let product_type = evidence
            .iter()
            .find(|e| e.kind == EvidenceKind::WindowsProductType)
            .expect("product type is recorded as evidence in its own right");
        assert_eq!(product_type.value, "3");
        assert_eq!(product_type.source, DiscoverySource::WindowsCredentialed);
        assert_eq!(product_type.confidence, Confidence::High);
    }

    #[test]
    fn a_joined_machine_records_its_domain_and_an_unjoined_one_its_workgroup() {
        let joined = WindowsFacts {
            part_of_domain: Some(true),
            domain: Some("corp.example".into()),
            ..Default::default()
        };
        let evidence = joined.to_evidence();
        let membership = evidence
            .iter()
            .find(|e| e.kind == EvidenceKind::DomainMembership)
            .unwrap();
        assert_eq!(membership.key, "domain");
        assert_eq!(membership.value, "corp.example");

        // Windows puts the string WORKGROUP in the Domain field of a machine
        // joined to nothing, so PartOfDomain is what decides.
        let standalone = WindowsFacts {
            part_of_domain: Some(false),
            domain: Some("WORKGROUP".into()),
            workgroup: Some("WORKGROUP".into()),
            ..Default::default()
        };
        let evidence = standalone.to_evidence();
        let membership = evidence
            .iter()
            .find(|e| e.kind == EvidenceKind::DomainMembership)
            .unwrap();
        assert_eq!(membership.key, "workgroup");
    }

    #[test]
    fn empty_values_never_become_evidence() {
        let facts = WindowsFacts {
            os_product: Some("   ".into()),
            hardware_model: Some(String::new()),
            ..Default::default()
        };
        let evidence = facts.to_evidence();
        assert!(!evidence.iter().any(|e| e.kind == EvidenceKind::OsProduct));
        assert!(!evidence.iter().any(|e| e.kind == EvidenceKind::Model));
    }

    #[test]
    fn an_empty_answer_is_recognisable_as_one() {
        assert!(WindowsFacts::default().is_empty());
        assert!(!WindowsFacts {
            product_type: Some(WindowsProductType::Server),
            ..Default::default()
        }
        .is_empty());
    }
}
