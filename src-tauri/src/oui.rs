//! MAC vendor (OUI) lookup backed by the full IEEE MA-L registry.
//!
//! The registry is embedded at compile time as a compact `PREFIX\tVendor`
//! table (`oui_data.tsv`, regenerated from <https://standards-oui.ieee.org>)
//! and parsed once into an in-memory map on first lookup. Resolving a vendor
//! is then an O(1) hash lookup keyed by the 24-bit OUI prefix.

use std::collections::HashMap;
use std::sync::OnceLock;

static OUI_DATA: &str = include_str!("oui_data.tsv");

fn table() -> &'static HashMap<u32, &'static str> {
    static TABLE: OnceLock<HashMap<u32, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::with_capacity(40_000);
        for line in OUI_DATA.lines() {
            if let Some((prefix, name)) = line.split_once('\t') {
                if let Ok(key) = u32::from_str_radix(prefix, 16) {
                    map.insert(key, name);
                }
            }
        }
        map
    })
}

/// The 24-bit OUI prefix of a MAC address as a numeric key (case-insensitive).
fn prefix_key(mac: &str) -> Option<u32> {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect();
    if hex.len() == 6 {
        u32::from_str_radix(&hex, 16).ok()
    } else {
        None
    }
}

/// Resolve a vendor name from a MAC address, if the prefix is registered.
pub fn vendor_for_mac(mac: &str) -> Option<String> {
    let key = prefix_key(mac)?;
    table().get(&key).map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_loads_and_resolves() {
        // The full IEEE MA-L registry has tens of thousands of entries.
        assert!(table().len() > 30_000, "OUI table looks too small");

        // Resolution is case- and separator-insensitive.
        assert!(vendor_for_mac("00:00:00:11:22:33").is_some());
        assert_eq!(
            vendor_for_mac("000000aabbcc"),
            vendor_for_mac("00-00-00-AA-BB-CC")
        );

        // Malformed / too-short input resolves to nothing.
        assert_eq!(vendor_for_mac("zz"), None);
        assert_eq!(vendor_for_mac(""), None);
    }
}
