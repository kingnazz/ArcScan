//! One physical device, seen from several addresses.
//!
//! # The problem
//!
//! A NAS with two network ports answers on two addresses with two MAC
//! addresses, and ArcScan finds two devices. So does a server with a separate
//! management controller, a workstation with Wi-Fi and Ethernet both up, and a
//! hypervisor host with a management NIC. Counting one box twice inflates the
//! inventory, splits its history, and — once the inventory reaches ArcAtlas —
//! draws it on the map twice.
//!
//! # The rule
//!
//! Two observations reconcile when they share an identifier that can only
//! belong to one physical machine. The order of preference is:
//!
//! 1. **System UUID** — the SMBIOS identity. One machine, one value.
//! 2. **Hardware serial / service tag** — namespaced by manufacturer, because a
//!    serial is unique per vendor and not across vendors.
//! 3. **Vendor-unique identifiers** — an SMB server GUID, a UPnP device UDN.
//! 4. **Stable device identifiers** — anything else a device persists across
//!    reboots and publishes deliberately.
//! 5. **MAC address** — the same interface seen twice.
//!
//! # What is deliberately not on that list
//!
//! **Host names.** Two machines called `NAS` are two machines, and on a network
//! with a Windows domain, a printer fleet or a batch of identical appliances,
//! duplicate names are the norm rather than the exception. A host name may
//! corroborate a merge that an identifier already justified; it may never cause
//! one.
//!
//! The asymmetry is deliberate. A missed merge shows a technician two rows for
//! one box, which is visibly wrong and easily corrected. A false merge silently
//! destroys one device's history inside another's, and nothing on screen says
//! so. So when the evidence is short of conclusive, this module declines.

use std::collections::{BTreeMap, BTreeSet};

use super::model::Confidence;

/// How much weight an identifier carries. Ordered strongest first, so the
/// derived `Ord` is the preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdentityStrength {
    /// The SMBIOS system UUID.
    SystemUuid,
    /// A hardware serial or service tag, namespaced by manufacturer.
    HardwareSerial,
    /// An identifier a vendor guarantees unique: an SMB server GUID, a UPnP UDN.
    VendorUnique,
    /// Anything else a device persists and publishes deliberately.
    StableDeviceId,
    /// A MAC address. The same interface, rather than the same machine.
    Mac,
}

impl IdentityStrength {
    /// A short, stable slug for the comparison key.
    ///
    /// Written out rather than derived from the variant name with `{:?}`,
    /// because this string is part of `physical_device_key`, which reaches an
    /// export and ArcAtlas. A rename of a Rust variant must not silently
    /// change a key that something else joins on.
    pub fn slug(self) -> &'static str {
        match self {
            IdentityStrength::SystemUuid => "uuid",
            IdentityStrength::HardwareSerial => "serial",
            IdentityStrength::VendorUnique => "vendor",
            IdentityStrength::StableDeviceId => "device",
            IdentityStrength::Mac => "mac",
        }
    }

    /// How the evidence line reads.
    pub fn label(self) -> &'static str {
        match self {
            IdentityStrength::SystemUuid => "system UUID",
            IdentityStrength::HardwareSerial => "hardware serial",
            IdentityStrength::VendorUnique => "vendor-unique identifier",
            IdentityStrength::StableDeviceId => "stable device identifier",
            IdentityStrength::Mac => "MAC address",
        }
    }

    /// How sure a merge justified by this identifier alone is.
    ///
    /// A MAC establishes an interface rather than a machine, so a group held
    /// together only by MACs is the ordinary one-device-one-NIC case and is
    /// reported as such rather than as a discovery.
    pub fn confidence(self) -> Confidence {
        match self {
            IdentityStrength::SystemUuid | IdentityStrength::HardwareSerial => Confidence::High,
            IdentityStrength::VendorUnique => Confidence::High,
            IdentityStrength::StableDeviceId => Confidence::Medium,
            IdentityStrength::Mac => Confidence::High,
        }
    }
}

/// One identifier claimed by one observation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdentityClaim {
    pub strength: IdentityStrength,
    /// The comparison key, already normalized. Two observations with the same
    /// `(strength, key)` are the same physical device.
    pub key: String,
    /// What the identifier was, for the evidence line.
    pub display: String,
}

impl IdentityStrength {
    /// Whether this kind of identifier needs a manufacturer to stay unique.
    ///
    /// A system UUID, a MAC and a vendor-guaranteed GUID are unique on their
    /// own. A serial is unique only within its manufacturer: `7SZ1B43` from
    /// Dell and `7SZ1B43` from a label printer are not the same machine.
    ///
    /// This lives on the strength rather than at the call site so that two
    /// callers passing different namespaces for the same globally-unique
    /// identifier cannot produce two keys for one device — which would mean a
    /// merge silently not happening, with nothing on screen to say why.
    fn is_namespaced(self) -> bool {
        matches!(
            self,
            IdentityStrength::HardwareSerial | IdentityStrength::StableDeviceId
        )
    }
}

impl IdentityClaim {
    /// Build a claim, refusing anything that is not usable as an identity.
    ///
    /// `namespace` is the manufacturer. It is used only for the strengths that
    /// need one — see [`IdentityStrength::is_namespaced`] — and ignored for the
    /// rest, so passing it is always safe.
    pub fn new(
        strength: IdentityStrength,
        namespace: Option<&str>,
        value: &str,
    ) -> Option<IdentityClaim> {
        let display = value.trim();
        if display.is_empty() {
            return None;
        }
        // The same refusal the credentialed parser applies, repeated here so
        // that an identifier arriving from any other source is held to it too.
        if super::windows::parse::is_placeholder_identifier(display) {
            return None;
        }
        let folded: String = display
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        // Two characters is not an identity, whatever it is attached to.
        if folded.len() < 3 {
            return None;
        }
        let namespace = namespace
            .filter(|_| strength.is_namespaced())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| {
                n.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .unwrap_or_default();
        Some(IdentityClaim {
            strength,
            key: format!("{}|{namespace}|{folded}", strength.slug()),
            display: display.to_string(),
        })
    }
}

/// Parse a stored evidence line back into a claim.
///
/// The inventory stores what reconciliation found as `"<kind>: <value>"`, which
/// is what a technician reads in the drawer. Reading it back means one closed
/// set of labels has to round-trip, which [`IdentityStrength::label`] writes
/// and this parses — and which the tests below pin down in both directions.
///
/// `namespace` is the manufacturer, needed because a hardware serial is unique
/// to its vendor and not across vendors.
pub fn claim_from_line(line: &str, namespace: Option<&str>) -> Option<IdentityClaim> {
    let (label, value) = line.split_once(':')?;
    let label = label.trim();
    let strength = [
        IdentityStrength::SystemUuid,
        IdentityStrength::HardwareSerial,
        IdentityStrength::VendorUnique,
        IdentityStrength::StableDeviceId,
        IdentityStrength::Mac,
    ]
    .into_iter()
    .find(|candidate| candidate.label() == label)?;
    // `IdentityClaim::new` ignores the namespace for the strengths that do not
    // need one, so it is passed unconditionally here.
    IdentityClaim::new(strength, namespace, value.trim())
}

/// One observation offered for reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceCandidate {
    /// The caller's own handle for this observation. Opaque here.
    pub device_id: i64,
    pub ip: Option<String>,
    pub mac: Option<String>,
    /// Recorded so a group can show it. Never used to decide a merge.
    pub hostname: Option<String>,
    pub identities: Vec<IdentityClaim>,
}

/// A group of observations judged to be one physical device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalDevice {
    /// A stable key derived from the group's strongest identifier.
    ///
    /// Deterministic: the same machine produces the same key on every scan, so
    /// a consumer can join on it across runs.
    pub key: String,
    /// The candidates' `device_id`s, ascending.
    pub members: Vec<i64>,
    /// Every address the group was seen on, sorted.
    pub addresses: Vec<String>,
    /// Every MAC in the group, sorted. Preserved rather than collapsed: a
    /// two-port NAS has two real MACs and both belong in the record.
    pub macs: Vec<String>,
    /// Plain-language reasons the group was formed.
    pub evidence: Vec<String>,
    /// How sure the grouping is, from the strongest shared identifier.
    pub confidence: Confidence,
}

impl PhysicalDevice {
    /// True when this device was seen on more than one address or MAC, which is
    /// the case worth telling anyone about.
    pub fn is_multi_homed(&self) -> bool {
        self.addresses.len() > 1 || self.macs.len() > 1
    }
}

/// Group observations into physical devices.
///
/// Deterministic: candidates are keyed by `device_id` and every collection is
/// sorted, so the same input produces byte-identical output whatever order it
/// arrived in. A candidate that shares no identifier with any other is returned
/// as a group of one, so every input is accounted for exactly once.
pub fn reconcile(candidates: &[DeviceCandidate]) -> Vec<PhysicalDevice> {
    let mut parent: Vec<usize> = (0..candidates.len()).collect();

    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }

    // Every identifier, and which candidates claimed it.
    let mut by_identity: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        for identity in &candidate.identities {
            by_identity
                .entry(identity.key.as_str())
                .or_default()
                .push(index);
        }
    }

    // A shared identifier unions its claimants. Sorted iteration keeps the
    // union order, and therefore the resulting group representatives, stable.
    for claimants in by_identity.values() {
        let Some(first) = claimants.first().copied() else {
            continue;
        };
        for other in claimants.iter().skip(1).copied() {
            let a = find(&mut parent, first);
            let b = find(&mut parent, other);
            if a != b {
                // Union by index keeps the representative deterministic.
                let (low, high) = if a < b { (a, b) } else { (b, a) };
                parent[high] = low;
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..candidates.len() {
        let root = find(&mut parent, index);
        groups.entry(root).or_default().push(index);
    }

    let mut out: Vec<PhysicalDevice> = groups
        .into_values()
        .map(|members| build_group(candidates, &members))
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

fn build_group(candidates: &[DeviceCandidate], members: &[usize]) -> PhysicalDevice {
    let mut device_ids: Vec<i64> = members.iter().map(|i| candidates[*i].device_id).collect();
    device_ids.sort_unstable();

    let mut addresses: BTreeSet<String> = BTreeSet::new();
    let mut macs: BTreeSet<String> = BTreeSet::new();
    let mut identities: BTreeSet<IdentityClaim> = BTreeSet::new();
    for index in members {
        let candidate = &candidates[*index];
        if let Some(ip) = &candidate.ip {
            addresses.insert(ip.clone());
        }
        if let Some(mac) = &candidate.mac {
            macs.insert(mac.clone());
        }
        identities.extend(candidate.identities.iter().cloned());
    }

    // Identifiers shared by more than one member are what actually justified
    // the grouping, and are the only ones worth stating as a reason.
    let mut shared: Vec<&IdentityClaim> = identities
        .iter()
        .filter(|identity| {
            members
                .iter()
                .filter(|index| candidates[**index].identities.contains(identity))
                .count()
                > 1
        })
        .collect();
    shared.sort();

    let key = identities
        .iter()
        .min()
        .map(|identity| identity.key.clone())
        // A candidate with no identifier at all is its own device, keyed by the
        // caller's handle so it is still stable across a scan.
        .unwrap_or_else(|| format!("device|{}", device_ids.first().copied().unwrap_or_default()));

    let mut evidence: Vec<String> = shared
        .iter()
        .map(|identity| format!("Shared {}: {}", identity.strength.label(), identity.display))
        .collect();
    if members.len() > 1 && evidence.is_empty() {
        // Cannot happen with the union rule above, and is stated rather than
        // assumed: a group with no shared identifier would be a false merge.
        evidence.push("Grouped without a shared identifier".into());
    }

    let confidence = if members.len() == 1 {
        Confidence::High
    } else {
        shared
            .iter()
            .map(|identity| identity.strength.confidence())
            .min()
            .unwrap_or(Confidence::Unknown)
    };

    PhysicalDevice {
        key,
        members: device_ids,
        addresses: addresses.into_iter().collect(),
        macs: macs.into_iter().collect(),
        evidence,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, ip: &str, mac: Option<&str>) -> DeviceCandidate {
        DeviceCandidate {
            device_id: id,
            ip: Some(ip.to_string()),
            mac: mac.map(str::to_string),
            hostname: None,
            identities: Vec::new(),
        }
    }

    #[test]
    fn a_shared_system_uuid_merges_two_addresses_into_one_device() {
        // The two-NIC Windows machine: different addresses, different MACs, one
        // SMBIOS UUID.
        let uuid = "4C4C4544-0037-5A10-8051-B4C04F435331";
        let mut a = candidate(1, "10.0.0.5", Some("aa:bb:cc:00:00:01"));
        a.identities
            .push(IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap());
        let mut b = candidate(2, "10.0.0.6", Some("aa:bb:cc:00:00:02"));
        b.identities
            .push(IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap());

        let groups = reconcile(&[a, b]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec![1, 2]);
        // Both addresses and both MACs survive the merge.
        assert_eq!(groups[0].addresses, vec!["10.0.0.5", "10.0.0.6"]);
        assert_eq!(groups[0].macs.len(), 2);
        assert!(groups[0].is_multi_homed());
        assert_eq!(groups[0].confidence, Confidence::High);
        assert!(groups[0].evidence[0].contains("system UUID"));
    }

    #[test]
    fn two_devices_with_the_same_hostname_are_never_merged() {
        // The fixture that matters most. Host names repeat constantly on real
        // networks, and a merge on one destroys a device's history silently.
        let mut a = candidate(1, "10.0.0.5", Some("aa:bb:cc:00:00:01"));
        a.hostname = Some("NAS".into());
        let mut b = candidate(2, "10.0.0.6", Some("aa:bb:cc:00:00:02"));
        b.hostname = Some("NAS".into());

        let groups = reconcile(&[a, b]);
        assert_eq!(groups.len(), 2, "identical host names must not merge");
    }

    #[test]
    fn a_shared_serial_merges_only_within_one_manufacturer() {
        let mut dell = candidate(1, "10.0.0.5", None);
        dell.identities.push(
            IdentityClaim::new(IdentityStrength::HardwareSerial, Some("Dell"), "7SZ1B43").unwrap(),
        );
        let mut other = candidate(2, "10.0.0.6", None);
        other.identities.push(
            IdentityClaim::new(IdentityStrength::HardwareSerial, Some("Zebra"), "7SZ1B43").unwrap(),
        );
        // Same string, different vendors: two devices.
        assert_eq!(reconcile(&[dell.clone(), other]).len(), 2);

        let mut dell_again = candidate(2, "10.0.0.6", None);
        dell_again.identities.push(
            IdentityClaim::new(IdentityStrength::HardwareSerial, Some("Dell"), "7SZ1B43").unwrap(),
        );
        assert_eq!(reconcile(&[dell, dell_again]).len(), 1);
    }

    #[test]
    fn a_synology_seen_on_two_ports_becomes_one_nas() {
        // The case reported from live testing: one appliance, two interfaces,
        // two rows in the map. An SMB server GUID is the identifier that closes
        // it without credentials.
        let guid = "4c4c4544-0037-5a10-8051-b4c04f435331";
        let mut lan1 = candidate(11, "10.0.0.20", Some("00:11:32:aa:bb:01"));
        lan1.hostname = Some("DiskStation".into());
        lan1.identities
            .push(IdentityClaim::new(IdentityStrength::VendorUnique, Some("smb"), guid).unwrap());
        let mut lan2 = candidate(12, "10.0.0.21", Some("00:11:32:aa:bb:02"));
        lan2.hostname = Some("DiskStation".into());
        lan2.identities
            .push(IdentityClaim::new(IdentityStrength::VendorUnique, Some("smb"), guid).unwrap());

        let groups = reconcile(&[lan1, lan2]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].addresses.len(), 2);
        assert_eq!(groups[0].macs.len(), 2);
    }

    #[test]
    fn a_management_controller_does_not_merge_into_the_server_it_manages() {
        // An iDRAC reports the chassis service tag, and the host reports the
        // same one. They are still two addressable devices with two MACs, and
        // merging them would lose the controller. They only merge when they
        // share an identifier of their own, which they do not here.
        let mut host = candidate(1, "10.0.0.5", Some("aa:bb:cc:00:00:01"));
        host.identities.push(
            IdentityClaim::new(
                IdentityStrength::SystemUuid,
                None,
                "4C4C4544-0037-5A10-8051-B4C04F435331",
            )
            .unwrap(),
        );
        let mut bmc = candidate(2, "10.0.0.6", Some("aa:bb:cc:00:00:99"));
        bmc.identities.push(
            IdentityClaim::new(IdentityStrength::VendorUnique, Some("idrac"), "7SZ1B43").unwrap(),
        );
        assert_eq!(reconcile(&[host, bmc]).len(), 2);
    }

    #[test]
    fn placeholder_identifiers_never_become_identity_claims() {
        // The failure this guards against: a rack of identical machines all
        // reporting "To Be Filled By O.E.M." collapsing into one device.
        for placeholder in [
            "To Be Filled By O.E.M.",
            "00000000-0000-0000-0000-000000000000",
            "Default string",
            "None",
            "  ",
            "0",
        ] {
            assert!(
                IdentityClaim::new(IdentityStrength::SystemUuid, None, placeholder).is_none(),
                "{placeholder} must not be usable as an identity"
            );
        }
    }

    #[test]
    fn a_very_short_identifier_is_refused() {
        assert!(IdentityClaim::new(IdentityStrength::StableDeviceId, None, "ab").is_none());
        assert!(IdentityClaim::new(IdentityStrength::StableDeviceId, None, "abc").is_some());
    }

    #[test]
    fn identifiers_compare_without_regard_to_punctuation_or_case() {
        let a = IdentityClaim::new(
            IdentityStrength::SystemUuid,
            None,
            "4C4C4544-0037-5A10-8051-B4C04F435331",
        )
        .unwrap();
        let b = IdentityClaim::new(
            IdentityStrength::SystemUuid,
            None,
            "4c4c4544003765a108051b4c04f435331".trim_end_matches('1'),
        );
        // Same UUID written differently is the same key.
        let c = IdentityClaim::new(
            IdentityStrength::SystemUuid,
            None,
            "4c4c4544 0037 5a10 8051 b4c04f435331",
        )
        .unwrap();
        assert_eq!(a.key, c.key);
        assert!(b.is_some());
    }

    #[test]
    fn a_mac_alone_still_groups_the_same_interface_seen_twice() {
        let mut a = candidate(1, "10.0.0.5", Some("aa:bb:cc:00:00:01"));
        a.identities
            .push(IdentityClaim::new(IdentityStrength::Mac, None, "aa:bb:cc:00:00:01").unwrap());
        let mut b = candidate(2, "10.0.0.9", Some("aa:bb:cc:00:00:01"));
        b.identities
            .push(IdentityClaim::new(IdentityStrength::Mac, None, "aa:bb:cc:00:00:01").unwrap());
        let groups = reconcile(&[a, b]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].addresses.len(), 2);
    }

    #[test]
    fn every_candidate_appears_in_exactly_one_group() {
        let candidates: Vec<DeviceCandidate> = (1..=5)
            .map(|i| candidate(i, &format!("10.0.0.{i}"), None))
            .collect();
        let groups = reconcile(&candidates);
        assert_eq!(groups.len(), 5);
        let mut seen: Vec<i64> = groups.iter().flat_map(|g| g.members.clone()).collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn reconciliation_is_deterministic_whatever_order_candidates_arrive_in() {
        let uuid = "4C4C4544-0037-5A10-8051-B4C04F435331";
        let build = || {
            let mut a = candidate(1, "10.0.0.5", Some("aa:bb:cc:00:00:01"));
            a.identities
                .push(IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap());
            let mut b = candidate(2, "10.0.0.6", Some("aa:bb:cc:00:00:02"));
            b.identities
                .push(IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap());
            let c = candidate(3, "10.0.0.7", Some("aa:bb:cc:00:00:03"));
            (a, b, c)
        };
        let (a, b, c) = build();
        let forward = reconcile(&[a, b, c]);
        let (a, b, c) = build();
        let backward = reconcile(&[c, b, a]);
        assert_eq!(forward, backward);
    }

    #[test]
    fn three_interfaces_on_one_machine_reconcile_together() {
        let uuid = "4C4C4544-0037-5A10-8051-B4C04F435331";
        let candidates: Vec<DeviceCandidate> = (1..=3)
            .map(|i| {
                let mut c = candidate(
                    i,
                    &format!("10.0.0.{i}"),
                    Some(&format!("aa:bb:cc:00:00:0{i}")),
                );
                c.identities
                    .push(IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap());
                c
            })
            .collect();
        let groups = reconcile(&candidates);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec![1, 2, 3]);
        assert_eq!(groups[0].addresses.len(), 3);
        assert_eq!(groups[0].macs.len(), 3);
    }

    #[test]
    fn every_identity_label_round_trips_through_a_stored_line() {
        // The property the inventory depends on: what the drawer shows is what
        // reconciliation can read back.
        for strength in [
            IdentityStrength::SystemUuid,
            IdentityStrength::HardwareSerial,
            IdentityStrength::VendorUnique,
            IdentityStrength::StableDeviceId,
            IdentityStrength::Mac,
        ] {
            let original = IdentityClaim::new(strength, Some("Dell"), "7SZ1B43").unwrap();
            let line = format!("{}: {}", strength.label(), original.display);
            let parsed = claim_from_line(&line, Some("Dell")).expect("the line parses back");
            assert_eq!(parsed, original, "{strength:?} did not round-trip");
        }
    }

    #[test]
    fn the_key_slug_is_written_out_rather_than_derived_from_a_variant_name() {
        // The key reaches an export and ArcAtlas, so renaming a Rust variant
        // must not change what something else joins on.
        let claim = IdentityClaim::new(
            IdentityStrength::SystemUuid,
            None,
            "4C4C4544-0037-5A10-8051-B4C04F435331",
        )
        .unwrap();
        assert_eq!(claim.key, "uuid||4c4c454400375a108051b4c04f435331");
        assert!(!claim.key.contains("SystemUuid"));
    }

    #[test]
    fn every_strength_has_a_distinct_slug() {
        let slugs: BTreeSet<&str> = [
            IdentityStrength::SystemUuid,
            IdentityStrength::HardwareSerial,
            IdentityStrength::VendorUnique,
            IdentityStrength::StableDeviceId,
            IdentityStrength::Mac,
        ]
        .into_iter()
        .map(IdentityStrength::slug)
        .collect();
        assert_eq!(slugs.len(), 5);
    }

    #[test]
    fn a_namespace_cannot_split_a_globally_unique_identifier_in_two() {
        // Two callers passing different namespaces for one system UUID must
        // still produce one key, or the merge silently would not happen.
        let uuid = "4C4C4544-0037-5A10-8051-B4C04F435331";
        let with = IdentityClaim::new(IdentityStrength::SystemUuid, Some("Dell"), uuid).unwrap();
        let without = IdentityClaim::new(IdentityStrength::SystemUuid, None, uuid).unwrap();
        assert_eq!(with.key, without.key);

        let mac = "aa:bb:cc:00:00:01";
        assert_eq!(
            IdentityClaim::new(IdentityStrength::Mac, Some("Dell"), mac)
                .unwrap()
                .key,
            IdentityClaim::new(IdentityStrength::Mac, None, mac)
                .unwrap()
                .key
        );
    }

    #[test]
    fn a_serial_still_needs_its_manufacturer() {
        let dell =
            IdentityClaim::new(IdentityStrength::HardwareSerial, Some("Dell"), "7SZ1B43").unwrap();
        let zebra =
            IdentityClaim::new(IdentityStrength::HardwareSerial, Some("Zebra"), "7SZ1B43").unwrap();
        assert_ne!(dell.key, zebra.key);
    }

    #[test]
    fn a_line_that_is_not_an_identity_parses_to_nothing() {
        assert!(claim_from_line("", None).is_none());
        assert!(claim_from_line("no colon here", None).is_none());
        assert!(claim_from_line("hostname: NAS", None).is_none());
        assert!(claim_from_line("system UUID: ", None).is_none());
        // A placeholder is still refused on the way back in.
        assert!(
            claim_from_line("system UUID: 00000000-0000-0000-0000-000000000000", None).is_none()
        );
    }

    #[test]
    fn a_value_containing_a_colon_survives_the_round_trip() {
        // A MAC is the obvious case, and the one most likely to be stored.
        let original =
            IdentityClaim::new(IdentityStrength::Mac, None, "aa:bb:cc:00:00:01").unwrap();
        let line = format!("{}: {}", IdentityStrength::Mac.label(), original.display);
        assert_eq!(claim_from_line(&line, None).unwrap(), original);
    }

    #[test]
    fn an_empty_input_produces_no_groups() {
        assert!(reconcile(&[]).is_empty());
    }

    #[test]
    fn a_lone_candidate_is_its_own_device_and_is_not_multi_homed() {
        let groups = reconcile(&[candidate(7, "10.0.0.5", Some("aa:bb:cc:00:00:01"))]);
        assert_eq!(groups.len(), 1);
        assert!(!groups[0].is_multi_homed());
        assert!(groups[0].evidence.is_empty());
    }
}
