//! Minimal embedded OUI (Organizationally Unique Identifier) lookup.
//!
//! This is a curated subset of common vendor prefixes — enough to label most
//! devices on a typical SMB/MSP network without bundling the full IEEE
//! registry. Lookups match the first three octets of a MAC address.

/// (prefix, vendor) where prefix is the upper-case hex of the first 3 octets
/// without separators, e.g. "FCECDA".
static OUI: &[(&str, &str)] = &[
    ("FCECDA", "Ubiquiti Inc"),
    ("245A4C", "Ubiquiti Inc"),
    ("788A20", "Ubiquiti Inc"),
    ("802AA8", "Ubiquiti Inc"),
    ("A483E7", "Apple, Inc."),
    ("AC87A3", "Apple, Inc."),
    ("F0189E", "Apple, Inc."),
    ("3C0754", "Apple, Inc."),
    ("001422", "Dell Inc."),
    ("001E4F", "Dell Inc."),
    ("B8AC6F", "Dell Inc."),
    ("F8BC12", "Dell Inc."),
    ("001B78", "Hewlett Packard"),
    ("3863BB", "Hewlett Packard"),
    ("9457A5", "Hewlett Packard Enterprise"),
    ("001AA1", "Cisco Systems"),
    ("00000C", "Cisco Systems"),
    ("3C0E23", "Cisco Systems"),
    ("00113C", "Cisco Meraki"),
    ("E0553D", "Cisco Meraki"),
    ("001132", "Synology Incorporated"),
    ("0011D8", "ASUSTek Computer"),
    ("2C56DC", "ASUSTek Computer"),
    ("DCA632", "Raspberry Pi Trading"),
    ("B827EB", "Raspberry Pi Foundation"),
    ("E45F01", "Raspberry Pi Trading"),
    ("94C691", "Intel Corporate"),
    ("8C1645", "Intel Corporate"),
    ("A0A8CD", "Intel Corporate"),
    ("000C29", "VMware, Inc."),
    ("005056", "VMware, Inc."),
    ("0050F2", "Microsoft Corporation"),
    ("00155D", "Microsoft Corporation (Hyper-V)"),
    ("F01FAF", "Dell Inc."),
    ("000F4B", "Oracle Corporation"),
    ("080027", "Oracle VirtualBox"),
    ("525400", "QEMU/KVM Virtual NIC"),
    ("D052A8", "Netgear"),
    ("204E7F", "Netgear"),
    ("A040A0", "Netgear"),
    ("C0C9E3", "Ubiquiti Inc"),
    ("002241", "Aruba Networks"),
    ("6CF37F", "Aruba Networks"),
    ("001801", "TP-Link Technologies"),
    ("50C7BF", "TP-Link Technologies"),
    ("F4F26D", "TP-Link Technologies"),
    ("EC086B", "TP-Link Technologies"),
    ("000D3A", "Microsoft Azure"),
    ("ACDE48", "Private (locally administered)"),
];

/// Normalize a MAC string to its 6-hex-digit OUI prefix (upper-case, no
/// separators). Returns `None` for malformed input.
fn prefix_of(mac: &str) -> Option<String> {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect::<String>()
        .to_uppercase();
    if hex.len() == 6 {
        Some(hex)
    } else {
        None
    }
}

/// Resolve a vendor name from a MAC address, if the prefix is known.
pub fn vendor_for_mac(mac: &str) -> Option<String> {
    let prefix = prefix_of(mac)?;
    OUI.iter()
        .find(|(p, _)| *p == prefix)
        .map(|(_, vendor)| vendor.to_string())
}
