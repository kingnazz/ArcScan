//! The one place ArcScan decides whether a VLAN fact is real.
//!
//! ArcAtlas accepts VLAN IDs in `1..=4094` and rejects a whole topology
//! handoff over a single out-of-range ID. SNMP produces out-of-range values in
//! normal operation: CDP reports `cdpCacheNativeVLAN = 0` for "this port has no
//! native VLAN", and any agent may answer a VLAN object with a Gauge32 or an
//! OID arc far wider than a VLAN ever is.
//!
//! The rule is deliberately narrow: a VLAN inside the range is preserved,
//! anything else is unknown. Nothing is clamped. Turning `0` into `1`, or
//! `65535` into `4094`, would invent a VLAN the network never reported, and a
//! wrong VLAN is worse than an absent one. Unknown stays unknown.
//!
//! Invalid values are dropped one fact at a time — an unusable native VLAN
//! removes the native VLAN and nothing else, and a bad entry in a tagged list
//! removes that entry and nothing else — so a link is never discarded over a
//! VLAN.

/// Lowest VLAN ID the ArcAtlas contract accepts.
pub const MIN_VLAN_ID: u16 = 1;
/// Highest VLAN ID the ArcAtlas contract accepts. 802.1Q reserves 4095.
pub const MAX_VLAN_ID: u16 = 4094;

/// The `vlan` label a port carries when it is a trunk rather than one VLAN.
pub const TRUNK_LABEL: &str = "trunk";

/// Is this an Ethernet VLAN ID ArcAtlas will accept?
pub fn is_valid_vlan_id(vlan: u16) -> bool {
    (MIN_VLAN_ID..=MAX_VLAN_ID).contains(&vlan)
}

/// Normalize one raw VLAN value into a VLAN fact.
///
/// The raw value is range-checked *before* it is narrowed, so a wide SNMP or
/// OID value cannot truncate into a plausible VLAN: `65546` is unknown, not
/// VLAN 10, and `4_294_971_390` is unknown, not VLAN 4094.
pub fn normalize_vlan_id<T: TryInto<u16>>(raw: T) -> Option<u16> {
    raw.try_into().ok().filter(|vlan| is_valid_vlan_id(*vlan))
}

/// Normalize a list of raw VLAN values, keeping every valid entry in order.
/// Invalid entries are filtered individually.
pub fn normalize_vlan_ids<T: TryInto<u16>, I: IntoIterator<Item = T>>(raw: I) -> Vec<u16> {
    raw.into_iter().filter_map(normalize_vlan_id).collect()
}

/// Normalize the `vlan` label on a link: `"trunk"` is preserved, a decimal
/// VLAN string is preserved only while it is in range, and anything else --
/// including `"0"`, `"4095"` and a value too wide to be a VLAN -- is unknown.
pub fn normalize_vlan_label(label: Option<&str>) -> Option<String> {
    let label = label?;
    if label == TRUNK_LABEL {
        return Some(TRUNK_LABEL.to_string());
    }
    // Parse wide, then range-check, so "65546" cannot become VLAN 10.
    let raw = label.parse::<u64>().ok()?;
    normalize_vlan_id(raw).map(|vlan| vlan.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_unknown_and_is_never_promoted_to_vlan_one() {
        assert_eq!(normalize_vlan_id(0u64), None);
        assert_eq!(normalize_vlan_id(0u32), None);
        assert_eq!(normalize_vlan_id(0u16), None);
    }

    #[test]
    fn the_range_boundaries_are_preserved_exactly() {
        assert_eq!(normalize_vlan_id(1u64), Some(1));
        assert_eq!(normalize_vlan_id(4094u64), Some(4094));
        assert_eq!(normalize_vlan_id(100u32), Some(100));
    }

    #[test]
    fn above_the_range_is_unknown_and_is_never_clamped_to_4094() {
        assert_eq!(normalize_vlan_id(4095u64), None);
        assert_eq!(normalize_vlan_id(4096u64), None);
        assert_eq!(normalize_vlan_id(u16::MAX), None);
    }

    #[test]
    fn wide_values_are_range_checked_before_they_are_narrowed() {
        // Each of these truncates into a plausible VLAN under `as u16`.
        assert_eq!(65_546u64 as u16, 10);
        assert_eq!(normalize_vlan_id(65_546u64), None);
        assert_eq!(normalize_vlan_id(65_546u32), None);
        assert_eq!(4_294_971_390u64 as u16, 4094);
        assert_eq!(normalize_vlan_id(4_294_971_390u64), None);
        assert_eq!(normalize_vlan_id(u32::MAX), None);
        assert_eq!(normalize_vlan_id(u64::MAX), None);
        assert_eq!(normalize_vlan_id(65_536u64), None);
    }

    #[test]
    fn a_list_drops_only_its_invalid_entries() {
        assert_eq!(
            normalize_vlan_ids(vec![0u32, 1, 100, 4094, 4095]),
            vec![1, 100, 4094]
        );
        assert_eq!(normalize_vlan_ids(vec![0u32, 4095]), Vec::<u16>::new());
        assert_eq!(normalize_vlan_ids(Vec::<u16>::new()), Vec::<u16>::new());
    }

    #[test]
    fn labels_keep_trunk_and_in_range_ids_only() {
        assert_eq!(
            normalize_vlan_label(Some("trunk")).as_deref(),
            Some("trunk")
        );
        assert_eq!(normalize_vlan_label(Some("1")).as_deref(), Some("1"));
        assert_eq!(normalize_vlan_label(Some("4094")).as_deref(), Some("4094"));
        assert_eq!(normalize_vlan_label(Some("0")), None);
        assert_eq!(normalize_vlan_label(Some("4095")), None);
        assert_eq!(normalize_vlan_label(Some("65546")), None);
        assert_eq!(normalize_vlan_label(Some("-1")), None);
        assert_eq!(normalize_vlan_label(Some("")), None);
        assert_eq!(normalize_vlan_label(Some("native")), None);
        assert_eq!(normalize_vlan_label(None), None);
    }
}
