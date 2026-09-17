//! Turn collected SNMP views plus the scan's inventory index into a
//! [`TopologySnapshot`].
//!
//! Rules that must not drift:
//! * LLDP/CDP produce `confirmed`. FDB on a single-MAC access port produces
//!   `strong`. Nothing else produces `confirmed`.
//! * Several weak clues never vote an `inferred` link up to `confirmed`.
//! * A port with many learned MACs is an uplink and does not mint endpoint links.
//! * The same link seen twice (A→B and B→A, or LLDP+FDB) is one connection,
//!   keeping the stronger protocol.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::collect::{is_unicast_mac, normalize_mac, DeviceView};
use super::display::INTERNET_NODE_ID;
use super::model::{
    EdgeHint, LogicalNode, TopologyConfidence, TopologyConnection, TopologyEdge, TopologyProtocol,
    TopologySnapshot, TopologyTarget, UnresolvedNode,
};

/// A port with this many relevant unicast MACs is treated as an uplink/trunk.
///
/// One MAC on an access port can be an endpoint. Two already might be a phone
/// and a PC, and many almost certainly is a downstream switch. The safety bar
/// is therefore two.
pub const UPLINK_MAC_THRESHOLD: usize = 2;

#[derive(Debug, Clone)]
pub struct InventoryIndex {
    by_ip: HashMap<String, i64>,
    by_mac: HashMap<String, i64>,
}

impl InventoryIndex {
    pub fn from_targets(targets: &[TopologyTarget]) -> Self {
        let mut idx = Self {
            by_ip: HashMap::new(),
            by_mac: HashMap::new(),
        };
        for t in targets {
            if let Some(id) = t.device_id {
                idx.by_ip.insert(t.ip.clone(), id);
                if let Some(mac) = t.mac.as_deref().and_then(normalize_mac) {
                    idx.by_mac.insert(mac, id);
                }
            }
        }
        idx
    }

    pub fn id_for_ip(&self, ip: &str) -> Option<i64> {
        self.by_ip.get(ip).copied()
    }

    pub fn resolve_neighbor(
        &self,
        chassis_id: Option<&str>,
        _sys_name: Option<&str>,
        management_address: Option<&str>,
    ) -> Option<i64> {
        // Hostname / sysName is evidence for an unknown node, never a
        // canonical inventory match. Duplicate hostnames (the case that
        // started this v1.9 work) would otherwise wire the wrong device.
        if let Some(ip) = management_address {
            if let Some(id) = self.by_ip.get(ip) {
                return Some(*id);
            }
        }
        if let Some(chassis) = chassis_id.and_then(normalize_mac) {
            if let Some(id) = self.by_mac.get(&chassis) {
                return Some(*id);
            }
        }
        None
    }

    pub fn resolve_mac(&self, mac: &str) -> Option<i64> {
        normalize_mac(mac).and_then(|m| self.by_mac.get(&m).copied())
    }
}

#[derive(Clone)]
struct Draft {
    from_device_id: Option<i64>,
    to_device_id: Option<i64>,
    from_unresolved: Option<UnresolvedNode>,
    to_unresolved: Option<UnresolvedNode>,
    from_logical_id: Option<String>,
    to_logical_id: Option<String>,
    from_port: Option<String>,
    to_port: Option<String>,
    protocol: TopologyProtocol,
    confidence: TopologyConfidence,
    speed_mbps: Option<u64>,
    vlan: Option<String>,
    native_vlan: Option<u16>,
    tagged_vlans: Vec<u16>,
    poe: Option<super::model::PoeInfo>,
    evidence: Vec<String>,
}

impl Draft {
    fn key(&self) -> String {
        // Undirected: two switches reporting each other collapse to one link.
        let a = endpoint_key(self.from_device_id, self.from_unresolved.as_ref());
        let b = endpoint_key(self.to_device_id, self.to_unresolved.as_ref());
        let pa = self.from_port.clone().unwrap_or_default();
        let pb = self.to_port.clone().unwrap_or_default();
        if a < b || (a == b && pa <= pb) {
            format!("{a}|{pa}|{b}|{pb}")
        } else {
            format!("{b}|{pb}|{a}|{pa}")
        }
    }
}

fn endpoint_key(id: Option<i64>, unresolved: Option<&UnresolvedNode>) -> String {
    if let Some(id) = id {
        format!("d:{id}")
    } else if let Some(u) = unresolved {
        format!("u:{}", u.id)
    } else {
        "u:unknown".into()
    }
}

pub fn correlate(
    views: &[DeviceView],
    targets: &[TopologyTarget],
    captured_at: &str,
) -> TopologySnapshot {
    correlate_with_edge(views, targets, captured_at, None)
}

pub fn correlate_with_edge(
    views: &[DeviceView],
    targets: &[TopologyTarget],
    captured_at: &str,
    edge_hint: Option<&EdgeHint>,
) -> TopologySnapshot {
    let index = InventoryIndex::from_targets(targets);
    let mut drafts: Vec<Draft> = Vec::new();
    let mut unknown: BTreeMap<String, UnresolvedNode> = BTreeMap::new();

    for view in views {
        let from_id = view
            .inventory_hint
            .or_else(|| index.id_for_ip(&view.target_ip.to_string()));
        let lldp_ports: BTreeSet<u32> = view
            .lldp_neighbors
            .iter()
            .map(|n| view.resolve_if_index(n.local_port_num))
            .collect();
        let cdp_ports: BTreeSet<u32> = view
            .cdp_neighbors
            .iter()
            .map(|n| view.resolve_if_index(n.if_index))
            .collect();

        for neigh in &view.lldp_neighbors {
            let local_if = view.resolve_if_index(neigh.local_port_num);
            let from_port = Some(view.port_name(local_if));
            let to_port = neigh.port_id.clone().or_else(|| neigh.port_desc.clone());
            let evidence = vec![format!(
                "LLDP neighbour on {} (local port {}): chassis {}, sysName {}, remote port {}",
                view.sys_name
                    .as_deref()
                    .unwrap_or(&view.target_ip.to_string()),
                from_port.clone().unwrap_or_else(|| local_if.to_string()),
                neigh.chassis_id.as_deref().unwrap_or("unknown"),
                neigh.sys_name.as_deref().unwrap_or("unknown"),
                to_port.as_deref().unwrap_or("unknown"),
            )];
            let (vlan, native, tagged) = view.vlan_for_port(local_if);
            let speed = view.iface(local_if).and_then(|i| i.speed_mbps);
            let poe = view.iface(local_if).and_then(|i| i.poe.clone());
            let to_id = index.resolve_neighbor(
                neigh.chassis_id.as_deref(),
                neigh.sys_name.as_deref(),
                neigh.management_address.as_deref(),
            );
            let to_unresolved = if to_id.is_none() {
                Some(unresolved_from_lldp(neigh))
            } else {
                None
            };
            drafts.push(Draft {
                from_device_id: from_id,
                to_device_id: to_id,
                from_unresolved: None,
                to_unresolved,
                from_logical_id: None,
                to_logical_id: None,
                from_port,
                to_port,
                protocol: TopologyProtocol::Lldp,
                confidence: TopologyConfidence::Confirmed,
                speed_mbps: speed,
                vlan,
                native_vlan: native,
                tagged_vlans: tagged,
                poe,
                evidence,
            });
        }

        for neigh in &view.cdp_neighbors {
            let local_if = view.resolve_if_index(neigh.if_index);
            if lldp_ports.contains(&local_if) {
                // LLDP already described this port. CDP is extra evidence, not
                // a second link, and must not change confidence.
                continue;
            }
            let from_port = Some(view.port_name(local_if));
            let to_port = neigh.device_port.clone();
            let evidence = vec![format!(
                "CDP neighbour on {}: deviceId {}, port {}",
                from_port.clone().unwrap_or_else(|| local_if.to_string()),
                neigh.device_id.as_deref().unwrap_or("unknown"),
                to_port.as_deref().unwrap_or("unknown"),
            )];
            let (vlan, native, tagged) = view.vlan_for_port(local_if);
            let native = neigh.native_vlan.or(native);
            let speed = view.iface(local_if).and_then(|i| i.speed_mbps);
            let poe = view.iface(local_if).and_then(|i| i.poe.clone());
            let to_id =
                index.resolve_neighbor(None, neigh.device_id.as_deref(), neigh.address.as_deref());
            let to_unresolved = if to_id.is_none() {
                Some(unresolved_from_cdp(neigh))
            } else {
                None
            };
            drafts.push(Draft {
                from_device_id: from_id,
                to_device_id: to_id,
                from_unresolved: None,
                to_unresolved,
                from_logical_id: None,
                to_logical_id: None,
                from_port,
                to_port,
                protocol: TopologyProtocol::Cdp,
                confidence: TopologyConfidence::Confirmed,
                speed_mbps: speed,
                vlan,
                native_vlan: native,
                tagged_vlans: tagged,
                poe,
                evidence,
            });
        }

        // FDB: group by port, refuse to mint endpoint links on uplinks.
        let own = view.own_macs();
        let mut by_port: BTreeMap<u32, Vec<&super::collect::FdbEntry>> = BTreeMap::new();
        for entry in &view.fdb {
            if !is_unicast_mac(&entry.mac) {
                continue;
            }
            if own.contains(&entry.mac) {
                continue;
            }
            by_port.entry(entry.if_index).or_default().push(entry);
        }
        for (if_index, entries) in by_port {
            if lldp_ports.contains(&if_index) || cdp_ports.contains(&if_index) {
                continue;
            }
            let unique_macs: BTreeSet<&str> = entries.iter().map(|e| e.mac.as_str()).collect();
            if unique_macs.len() >= UPLINK_MAC_THRESHOLD {
                continue;
            }
            if unique_macs.len() != 1 {
                continue;
            }
            if !view.is_access_port(if_index) {
                // A trunk with one MAC is still not a trustworthy endpoint
                // attachment; it is more likely a quiet uplink.
                continue;
            }
            let mac = *unique_macs.iter().next().unwrap();
            let Some(to_id) = index.resolve_mac(mac) else {
                // An unmatched MAC on an access port is an unknown endpoint,
                // not a fabricated device.
                let node = unresolved_mac(mac);
                let (vlan, native, tagged) = view.vlan_for_port(if_index);
                drafts.push(Draft {
                    from_device_id: from_id,
                    to_device_id: None,
                    from_unresolved: None,
                    to_unresolved: Some(node),
                    from_logical_id: None,
                    to_logical_id: None,
                    from_port: Some(view.port_name(if_index)),
                    to_port: None,
                    protocol: TopologyProtocol::Fdb,
                    confidence: TopologyConfidence::Strong,
                    speed_mbps: view.iface(if_index).and_then(|i| i.speed_mbps),
                    vlan,
                    native_vlan: native,
                    tagged_vlans: tagged,
                    poe: view.iface(if_index).and_then(|i| i.poe.clone()),
                    evidence: vec![format!(
                        "Exactly one unicast MAC ({mac}) learned on access port {}",
                        view.port_name(if_index)
                    )],
                });
                continue;
            };
            let ip_hint = view
                .arp
                .iter()
                .find(|a| normalize_mac(&a.mac).as_deref() == Some(mac))
                .map(|a| a.ip.clone());
            let mut evidence = vec![format!(
                "Exactly one unicast MAC ({mac}) learned on access port {}",
                view.port_name(if_index)
            )];
            if let Some(ip) = ip_hint {
                evidence.push(format!("ARP maps {mac} to {ip}"));
            }
            let (vlan, native, tagged) = view.vlan_for_port(if_index);
            drafts.push(Draft {
                from_device_id: from_id,
                to_device_id: Some(to_id),
                from_unresolved: None,
                to_unresolved: None,
                from_logical_id: None,
                to_logical_id: None,
                from_port: Some(view.port_name(if_index)),
                to_port: None,
                protocol: TopologyProtocol::Fdb,
                confidence: TopologyConfidence::Strong,
                speed_mbps: view.iface(if_index).and_then(|i| i.speed_mbps),
                vlan,
                native_vlan: native,
                tagged_vlans: tagged,
                poe: view.iface(if_index).and_then(|i| i.poe.clone()),
                evidence,
            });
        }
    }

    let merged = merge_drafts(drafts);
    let mut connections = Vec::new();
    for draft in merged {
        if let Some(node) = draft.from_unresolved.clone() {
            unknown.entry(node.id.clone()).or_insert(node);
        }
        if let Some(node) = draft.to_unresolved.clone() {
            unknown.entry(node.id.clone()).or_insert(node);
        }
        connections.push(TopologyConnection {
            from_device_id: draft.from_device_id,
            to_device_id: draft.to_device_id,
            from_unresolved_id: draft.from_unresolved.as_ref().map(|n| n.id.clone()),
            to_unresolved_id: draft.to_unresolved.as_ref().map(|n| n.id.clone()),
            from_logical_id: draft.from_logical_id,
            to_logical_id: draft.to_logical_id,
            from_port: draft.from_port,
            to_port: draft.to_port,
            kind: "ethernet".into(),
            protocol: draft.protocol.as_str().to_string(),
            confidence: draft.confidence,
            speed_mbps: draft.speed_mbps,
            vlan: draft.vlan,
            native_vlan: draft.native_vlan,
            tagged_vlans: draft.tagged_vlans,
            poe: draft.poe,
            evidence: draft.evidence,
        });
    }

    let (logical_nodes, edge) =
        attach_wan_edge(&mut connections, &mut unknown, views, &index, edge_hint);

    connections.sort_by(|a, b| {
        (a.from_device_id, a.to_device_id, &a.from_port, &a.to_port).cmp(&(
            b.from_device_id,
            b.to_device_id,
            &b.from_port,
            &b.to_port,
        ))
    });

    TopologySnapshot {
        captured_at: captured_at.to_string(),
        connections,
        unknown_nodes: unknown.into_values().collect(),
        logical_nodes,
        edge,
    }
}

fn attach_wan_edge(
    connections: &mut Vec<TopologyConnection>,
    unknown: &mut BTreeMap<String, UnresolvedNode>,
    views: &[DeviceView],
    index: &InventoryIndex,
    hint: Option<&EdgeHint>,
) -> (Vec<LogicalNode>, Option<TopologyEdge>) {
    let Some(hint) = hint else {
        return (Vec::new(), None);
    };
    let Some(hit) = resolve_gateway(views, index, hint) else {
        return (Vec::new(), None);
    };
    let gateway_id = hit.device_id;
    let gateway_ip = hit.ip;
    let gateway_mac = hit.mac;
    let confidence = hit.confidence;
    let mut evidence = hit.evidence;

    let via = find_ont_or_modem(views, index, gateway_id);
    if let Some(WanVia::Unresolved(ont)) = via.as_ref() {
        unknown.entry(ont.id.clone()).or_insert_with(|| ont.clone());
    }
    let internet = LogicalNode::internet();
    let uplink = match via.as_ref() {
        Some(WanVia::Unresolved(ont)) => {
            evidence.push(format!(
                "LLDP/CDP neighbour {} sits between the default gateway and the WAN.",
                ont.sys_name
                    .as_deref()
                    .or(ont.management_address.as_deref())
                    .unwrap_or(ont.id.as_str())
            ));
            wan_uplink(None, Some(ont.id.clone()), confidence, evidence.clone())
        }
        Some(WanVia::Device { id, label }) => {
            evidence.push(format!(
                "LLDP/CDP neighbour {label} (inventory device {id}) sits between the default gateway and the WAN."
            ));
            wan_uplink(Some(*id), None, confidence, evidence.clone())
        }
        None => wan_uplink(Some(gateway_id), None, confidence, evidence.clone()),
    };
    connections.push(uplink.clone());
    let edge = TopologyEdge {
        gateway_device_id: Some(gateway_id),
        gateway_ip,
        gateway_mac,
        internet: internet.clone(),
        via_unresolved_id: match via.as_ref() {
            Some(WanVia::Unresolved(ont)) => Some(ont.id.clone()),
            _ => None,
        },
        via_device_id: match via.as_ref() {
            Some(WanVia::Device { id, .. }) => Some(*id),
            _ => None,
        },
        uplink,
        confidence,
        evidence,
    };
    (vec![internet], Some(edge))
}

fn wan_uplink(
    to_device_id: Option<i64>,
    to_unresolved_id: Option<String>,
    confidence: TopologyConfidence,
    evidence: Vec<String>,
) -> TopologyConnection {
    TopologyConnection {
        from_device_id: None,
        to_device_id,
        from_unresolved_id: None,
        to_unresolved_id,
        from_logical_id: Some(INTERNET_NODE_ID.into()),
        to_logical_id: None,
        from_port: None,
        to_port: None,
        kind: "wan".into(),
        protocol: "default-route".into(),
        confidence,
        speed_mbps: None,
        vlan: None,
        native_vlan: None,
        tagged_vlans: Vec::new(),
        poe: None,
        evidence,
    }
}

struct GatewayHit {
    device_id: i64,
    ip: Option<String>,
    mac: Option<String>,
    confidence: TopologyConfidence,
    evidence: Vec<String>,
}

enum WanVia {
    Device { id: i64, label: String },
    Unresolved(UnresolvedNode),
}

fn resolve_gateway(
    views: &[DeviceView],
    index: &InventoryIndex,
    hint: &EdgeHint,
) -> Option<GatewayHit> {
    let ip = hint
        .gateway_ip
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mac = hint.gateway_mac.as_deref().and_then(normalize_mac);

    let by_ip = ip.and_then(|addr| index.id_for_ip(addr));
    let by_mac = mac.as_deref().and_then(|m| index.resolve_mac(m));

    // ARP on any answering device can confirm the gateway MAC when the host
    // process only learned the IP.
    let arp_mac = ip.and_then(|addr| {
        views.iter().find_map(|view| {
            view.arp
                .iter()
                .find(|entry| entry.ip == addr)
                .and_then(|entry| normalize_mac(&entry.mac))
        })
    });
    let by_arp = arp_mac.as_deref().and_then(|m| index.resolve_mac(m));

    let (id, confidence) = match (by_ip, by_mac, by_arp) {
        (Some(a), Some(b), _) if a == b => (a, TopologyConfidence::Strong),
        (Some(a), None, Some(c)) if a == c => (a, TopologyConfidence::Strong),
        (Some(a), None, None) => (a, TopologyConfidence::Inferred),
        (None, Some(b), _) => (b, TopologyConfidence::Inferred),
        (None, None, Some(c)) => (c, TopologyConfidence::Inferred),
        (Some(_), Some(_), _) => return None, // IP and MAC name two different devices
        _ => return None,
    };

    let mut evidence = Vec::new();
    if let Some(addr) = ip {
        evidence.push(format!(
            "The scanner's default route is {addr}, which matches inventory device {id}."
        ));
    }
    if let Some(m) = mac.as_deref().or(arp_mac.as_deref()) {
        evidence.push(format!(
            "Default-gateway MAC {m} matches inventory device {id}."
        ));
    }
    Some(GatewayHit {
        device_id: id,
        ip: ip.map(str::to_string),
        mac: mac.or(arp_mac),
        confidence,
        evidence,
    })
}

fn find_ont_or_modem(
    views: &[DeviceView],
    index: &InventoryIndex,
    gateway_id: i64,
) -> Option<WanVia> {
    for view in views {
        let from_id = view
            .inventory_hint
            .or_else(|| index.id_for_ip(&view.target_ip.to_string()));
        if from_id != Some(gateway_id) {
            continue;
        }
        for neigh in &view.lldp_neighbors {
            if !looks_like_ont_or_modem(neigh.sys_name.as_deref(), neigh.sys_desc.as_deref(), None)
            {
                continue;
            }
            if let Some(id) = index.resolve_neighbor(
                neigh.chassis_id.as_deref(),
                neigh.sys_name.as_deref(),
                neigh.management_address.as_deref(),
            ) {
                if id != gateway_id {
                    let label = neigh
                        .sys_name
                        .clone()
                        .or_else(|| neigh.management_address.clone())
                        .unwrap_or_else(|| format!("device {id}"));
                    return Some(WanVia::Device { id, label });
                }
                continue;
            }
            return Some(WanVia::Unresolved(unresolved_from_lldp(neigh)));
        }
        for neigh in &view.cdp_neighbors {
            if !looks_like_ont_or_modem(neigh.device_id.as_deref(), neigh.platform.as_deref(), None)
            {
                continue;
            }
            if let Some(id) =
                index.resolve_neighbor(None, neigh.device_id.as_deref(), neigh.address.as_deref())
            {
                if id != gateway_id {
                    let label = neigh
                        .device_id
                        .clone()
                        .or_else(|| neigh.address.clone())
                        .unwrap_or_else(|| format!("device {id}"));
                    return Some(WanVia::Device { id, label });
                }
                continue;
            }
            return Some(WanVia::Unresolved(unresolved_from_cdp(neigh)));
        }
    }
    None
}

fn looks_like_ont_or_modem(a: Option<&str>, b: Option<&str>, c: Option<&str>) -> bool {
    let haystack = format!(
        "{} {} {}",
        a.unwrap_or(""),
        b.unwrap_or(""),
        c.unwrap_or("")
    )
    .to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "ont",
        "gpon",
        "xgpon",
        "olt",
        "optical network",
        "optical network terminal",
        "cable modem",
        "docsis",
        "fibre modem",
        "fiber modem",
        "dsl modem",
        "modem",
    ];
    NEEDLES.iter().any(|n| haystack.contains(n))
}

fn merge_drafts(drafts: Vec<Draft>) -> Vec<Draft> {
    let mut by_key: BTreeMap<String, Draft> = BTreeMap::new();
    for draft in drafts {
        let key = draft.key();
        match by_key.get_mut(&key) {
            None => {
                by_key.insert(key, draft);
            }
            Some(existing) => {
                if draft.protocol.rank() > existing.protocol.rank()
                    || (draft.protocol.rank() == existing.protocol.rank()
                        && draft.confidence > existing.confidence)
                {
                    let mut kept = draft;
                    let mut evidence = existing.evidence.clone();
                    evidence.extend(kept.evidence.iter().cloned());
                    evidence.sort();
                    evidence.dedup();
                    kept.evidence = evidence;
                    if kept.speed_mbps.is_none() {
                        kept.speed_mbps = existing.speed_mbps;
                    }
                    if kept.poe.is_none() {
                        kept.poe = existing.poe.clone();
                    }
                    if kept.vlan.is_none() {
                        kept.vlan = existing.vlan.clone();
                        kept.native_vlan = existing.native_vlan;
                        kept.tagged_vlans = existing.tagged_vlans.clone();
                    }
                    // Never raise confidence above what the winning protocol
                    // itself earned. LLDP stays confirmed; FDB stays strong.
                    *existing = kept;
                } else {
                    for line in draft.evidence {
                        if !existing.evidence.contains(&line) {
                            existing.evidence.push(line);
                        }
                    }
                    if existing.speed_mbps.is_none() {
                        existing.speed_mbps = draft.speed_mbps;
                    }
                    if existing.poe.is_none() {
                        existing.poe = draft.poe;
                    }
                }
            }
        }
    }
    by_key.into_values().collect()
}

fn unresolved_from_lldp(neigh: &super::collect::LldpNeighbor) -> UnresolvedNode {
    let id = unresolved_id(
        neigh.chassis_id.as_deref(),
        neigh.sys_name.as_deref(),
        neigh.management_address.as_deref(),
    );
    UnresolvedNode {
        id,
        chassis_id: neigh.chassis_id.clone(),
        sys_name: neigh.sys_name.clone(),
        management_address: neigh.management_address.clone(),
        reason: "LLDP neighbour is not present in this scan's inventory.".into(),
        source: "lldp".into(),
    }
}

fn unresolved_from_cdp(neigh: &super::collect::CdpNeighbor) -> UnresolvedNode {
    let id = unresolved_id(
        neigh.device_id.as_deref(),
        neigh.device_id.as_deref(),
        neigh.address.as_deref(),
    );
    UnresolvedNode {
        id,
        chassis_id: neigh.device_id.clone(),
        sys_name: neigh.device_id.clone(),
        management_address: neigh.address.clone(),
        reason: "CDP neighbour is not present in this scan's inventory.".into(),
        source: "cdp".into(),
    }
}

fn unresolved_mac(mac: &str) -> UnresolvedNode {
    let id = format!("unknown:mac:{}", mac.to_ascii_lowercase().replace(':', ""));
    UnresolvedNode {
        id,
        chassis_id: Some(mac.to_string()),
        sys_name: None,
        management_address: None,
        reason: "A MAC was learned on an access port, but no inventory device has that address."
            .into(),
        source: "fdb".into(),
    }
}

fn unresolved_id(chassis: Option<&str>, sys_name: Option<&str>, ip: Option<&str>) -> String {
    if let Some(mac) = chassis.and_then(normalize_mac) {
        return format!(
            "unknown:chassis:{}",
            mac.to_ascii_lowercase().replace(':', "")
        );
    }
    if let Some(ip) = ip {
        return format!("unknown:ip:{ip}");
    }
    if let Some(name) = sys_name {
        return format!(
            "unknown:name:{}",
            name.trim().to_ascii_lowercase().replace(' ', "-")
        );
    }
    if let Some(chassis) = chassis {
        return format!(
            "unknown:chassis:{}",
            chassis.trim().to_ascii_lowercase().replace([' ', ':'], "-")
        );
    }
    "unknown:unidentified".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{ArpEntry, CdpNeighbor, FdbEntry, Iface, LldpNeighbor};
    use crate::model::PoeInfo;
    use std::net::Ipv4Addr;

    fn target(id: i64, ip: &str, mac: &str, name: &str) -> TopologyTarget {
        TopologyTarget {
            ip: ip.into(),
            mac: Some(mac.into()),
            device_id: Some(id),
            hostname: Some(name.into()),
            detected_name: Some(name.into()),
        }
    }

    fn switch_view() -> DeviceView {
        let mut v = DeviceView::new(Ipv4Addr::new(192, 168, 1, 2), Some(2));
        v.sys_name = Some("core-sw".into());
        v.bridge_address = Some("00:1A:2B:00:00:02".into());
        v.chassis_id = Some("00:1A:2B:00:00:02".into());
        for idx in [7, 12, 20, 24, 36, 48] {
            v.interfaces.insert(
                idx,
                Iface {
                    index: idx,
                    name: Some(format!("Port {idx}")),
                    descr: Some(format!("GigabitEthernet0/{idx}")),
                    alias: None,
                    mac: None,
                    if_type: Some(6),
                    admin_status: Some(1),
                    oper_status: Some(1),
                    speed_mbps: Some(1000),
                    poe: None,
                },
            );
            v.pvid.insert(idx, 10);
        }
        v.pvid.insert(48, 10);
        v.tagged.insert(48, BTreeSet::from([10, 20, 30]));
        v.pvid.insert(12, 20);
        v.interfaces.get_mut(&12).unwrap().poe = Some(PoeInfo {
            enabled: true,
            watts: Some(8.2),
        });
        v
    }

    fn site_targets() -> Vec<TopologyTarget> {
        vec![
            target(1, "192.168.1.1", "00:20:AA:00:00:01", "sonicwall"),
            target(2, "192.168.1.2", "00:1A:2B:00:00:02", "core-sw"),
            target(3, "192.168.1.12", "00:1A:2B:00:00:12", "u7-pro"),
            target(4, "192.168.1.20", "00:11:32:00:00:20", "synology"),
            target(5, "192.168.1.50", "AA:BB:CC:00:00:50", "workstation"),
            target(6, "192.168.1.3", "00:1A:2B:00:00:03", "access-sw"),
        ]
    }

    #[test]
    fn lldp_direct_switch_neighbor() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 48,
            chassis_id: Some("00:20:AA:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("X0".into()),
            port_desc: Some("X0".into()),
            sys_name: Some("sonicwall".into()),
            sys_desc: None,
            management_address: Some("192.168.1.1".into()),
        });
        let snap = correlate(&[core], &site_targets(), "2026-09-16T12:00:00Z");
        let link = snap
            .connections
            .iter()
            .find(|c| c.from_device_id == Some(2) && c.to_device_id == Some(1))
            .expect("core → firewall");
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
        assert_eq!(link.protocol, "lldp");
        assert_eq!(link.from_port.as_deref(), Some("Port 48"));
        assert_eq!(link.to_port.as_deref(), Some("X0"));
        assert_eq!(link.vlan.as_deref(), Some("trunk"));
        assert_eq!(link.native_vlan, Some(10));
        assert_eq!(link.tagged_vlans, vec![10, 20, 30]);
        assert_eq!(link.speed_mbps, Some(1000));
        assert!(link.evidence.iter().any(|e| e.contains("LLDP")));
    }

    #[test]
    fn lldp_ap_neighbor_with_poe() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:1A:2B:00:00:12".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("u7-pro".into()),
            sys_desc: None,
            management_address: Some("192.168.1.12".into()),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = snap
            .connections
            .iter()
            .find(|c| c.to_device_id == Some(3))
            .unwrap();
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
        assert_eq!(link.protocol, "lldp");
        assert_eq!(link.vlan.as_deref(), Some("20"));
        let poe = link.poe.as_ref().expect("poe");
        assert!(poe.enabled);
        assert_eq!(poe.watts, Some(8.2));
    }

    #[test]
    fn cdp_neighbor() {
        let mut core = switch_view();
        core.cdp_neighbors.push(CdpNeighbor {
            if_index: 36,
            device_id: Some("access-sw".into()),
            device_port: Some("GigabitEthernet0/1".into()),
            platform: Some("Cisco IOS".into()),
            address: Some("192.168.1.3".into()),
            native_vlan: Some(10),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = snap
            .connections
            .iter()
            .find(|c| c.to_device_id == Some(6))
            .unwrap();
        assert_eq!(link.protocol, "cdp");
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
        assert_eq!(link.to_port.as_deref(), Some("GigabitEthernet0/1"));
    }

    #[test]
    fn single_mac_access_port_is_strong() {
        let mut core = switch_view();
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        core.arp.push(ArpEntry {
            ip: "192.168.1.50".into(),
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: Some(7),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = snap
            .connections
            .iter()
            .find(|c| c.to_device_id == Some(5))
            .unwrap();
        assert_eq!(link.confidence, TopologyConfidence::Strong);
        assert_eq!(link.protocol, "fdb");
        assert_eq!(link.from_port.as_deref(), Some("Port 7"));
        assert!(link.evidence.iter().any(|e| e.contains("ARP")));
    }

    #[test]
    fn multi_mac_uplink_does_not_create_fake_endpoint_links() {
        let mut core = switch_view();
        for (i, mac) in [
            "AA:AA:AA:00:00:01",
            "AA:AA:AA:00:00:02",
            "AA:AA:AA:00:00:03",
            "AA:BB:CC:00:00:50",
        ]
        .into_iter()
        .enumerate()
        {
            core.fdb.push(FdbEntry {
                mac: mac.into(),
                if_index: 24,
                vlan: Some(10),
            });
            let _ = i;
        }
        let snap = correlate(&[core], &site_targets(), "t");
        assert!(
            snap.connections.is_empty(),
            "uplink FDB must not mint endpoint links, got {:?}",
            snap.connections
        );
    }

    #[test]
    fn vlan_trunk_and_access() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 48,
            chassis_id: Some("00:20:AA:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("X0".into()),
            port_desc: None,
            sys_name: Some("sonicwall".into()),
            sys_desc: None,
            management_address: Some("192.168.1.1".into()),
        });
        core.fdb.push(FdbEntry {
            mac: "00:11:32:00:00:20".into(),
            if_index: 20,
            vlan: Some(10),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let trunk = snap
            .connections
            .iter()
            .find(|c| c.from_port.as_deref() == Some("Port 48"))
            .unwrap();
        assert_eq!(trunk.vlan.as_deref(), Some("trunk"));
        let access = snap
            .connections
            .iter()
            .find(|c| c.from_port.as_deref() == Some("Port 20"))
            .unwrap();
        assert_eq!(access.vlan.as_deref(), Some("10"));
        assert_eq!(access.native_vlan, Some(10));
        assert!(access.tagged_vlans.is_empty());
    }

    #[test]
    fn missing_lldp_falls_back_to_fdb_without_promoting_confidence() {
        let mut core = switch_view();
        core.fdb.push(FdbEntry {
            mac: "00:11:32:00:00:20".into(),
            if_index: 20,
            vlan: Some(10),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = &snap.connections[0];
        assert_eq!(link.protocol, "fdb");
        assert_eq!(link.confidence, TopologyConfidence::Strong);
        assert_ne!(link.confidence, TopologyConfidence::Confirmed);
    }

    #[test]
    fn cyclic_neighbors_are_deduplicated() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("00:1A:2B:00:00:03".into()),
            chassis_subtype: Some(4),
            port_id: Some("Port 1".into()),
            port_desc: None,
            sys_name: Some("access-sw".into()),
            sys_desc: None,
            management_address: Some("192.168.1.3".into()),
        });
        let mut access = DeviceView::new(Ipv4Addr::new(192, 168, 1, 3), Some(6));
        access.sys_name = Some("access-sw".into());
        access.chassis_id = Some("00:1A:2B:00:00:03".into());
        access.interfaces.insert(
            1,
            Iface {
                index: 1,
                name: Some("Port 1".into()),
                descr: None,
                alias: None,
                mac: None,
                if_type: Some(6),
                admin_status: Some(1),
                oper_status: Some(1),
                speed_mbps: Some(1000),
                poe: None,
            },
        );
        access.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 1,
            chassis_id: Some("00:1A:2B:00:00:02".into()),
            chassis_subtype: Some(4),
            port_id: Some("Port 36".into()),
            port_desc: None,
            sys_name: Some("core-sw".into()),
            sys_desc: None,
            management_address: Some("192.168.1.2".into()),
        });
        let snap = correlate(&[core, access], &site_targets(), "t");
        let pair = snap
            .connections
            .iter()
            .filter(|c| {
                matches!(
                    (c.from_device_id, c.to_device_id),
                    (Some(2), Some(6)) | (Some(6), Some(2))
                )
            })
            .count();
        assert_eq!(pair, 1, "cyclic LLDP must collapse to one connection");
    }

    #[test]
    fn lldp_beats_fdb_on_the_same_port() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:1A:2B:00:00:12".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("u7-pro".into()),
            sys_desc: None,
            management_address: Some("192.168.1.12".into()),
        });
        core.fdb.push(FdbEntry {
            mac: "00:1A:2B:00:00:12".into(),
            if_index: 12,
            vlan: Some(20),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let links: Vec<_> = snap
            .connections
            .iter()
            .filter(|c| c.to_device_id == Some(3) || c.from_device_id == Some(3))
            .collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].protocol, "lldp");
        assert_eq!(links[0].confidence, TopologyConfidence::Confirmed);
    }

    #[test]
    fn inferred_never_upgrades_to_confirmed() {
        // Two FDB observations of the same MAC still stay strong, never confirmed.
        let mut core = switch_view();
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        let mut access = DeviceView::new(Ipv4Addr::new(192, 168, 1, 3), Some(6));
        access.sys_name = Some("access-sw".into());
        access.interfaces.insert(
            7,
            Iface {
                index: 7,
                name: Some("Port 7".into()),
                descr: None,
                alias: None,
                mac: None,
                if_type: Some(6),
                admin_status: Some(1),
                oper_status: Some(1),
                speed_mbps: Some(1000),
                poe: None,
            },
        );
        access.pvid.insert(7, 10);
        access.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        let snap = correlate(&[core, access], &site_targets(), "t");
        for link in &snap.connections {
            assert_ne!(link.confidence, TopologyConfidence::Confirmed);
            assert_eq!(link.protocol, "fdb");
        }
    }

    #[test]
    fn unknown_lldp_neighbour_is_preserved_without_a_vendor() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("DE:AD:BE:EF:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("Gi0/1".into()),
            port_desc: None,
            sys_name: Some("mystery-sw".into()),
            sys_desc: None,
            management_address: None,
        });
        let snap = correlate(&[core], &site_targets(), "t");
        assert_eq!(snap.unknown_nodes.len(), 1);
        let node = &snap.unknown_nodes[0];
        assert!(node.id.starts_with("unknown:"));
        assert_eq!(node.sys_name.as_deref(), Some("mystery-sw"));
        assert!(!serde_json::to_string(node)
            .unwrap()
            .to_ascii_lowercase()
            .contains("cisco"));
        let link = &snap.connections[0];
        assert_eq!(link.to_device_id, None);
        assert_eq!(link.to_unresolved_id.as_deref(), Some(node.id.as_str()));
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
    }

    #[test]
    fn hostname_alone_does_not_resolve_a_neighbor() {
        let mut core = switch_view();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("DE:AD:BE:EF:00:99".into()),
            chassis_subtype: Some(4),
            port_id: Some("Gi0/1".into()),
            port_desc: None,
            sys_name: Some("access-sw".into()),
            sys_desc: None,
            management_address: None,
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = snap
            .connections
            .iter()
            .find(|c| c.from_device_id == Some(2))
            .expect("LLDP evidence is kept");
        assert_eq!(
            link.to_device_id, None,
            "sysName must not pick an inventory device"
        );
        assert!(link.to_unresolved_id.is_some());
        assert_eq!(snap.unknown_nodes.len(), 1);
        assert_eq!(snap.unknown_nodes[0].sys_name.as_deref(), Some("access-sw"));
    }

    #[test]
    fn duplicate_hostnames_stay_unresolved_unless_ip_or_mac_disambiguates() {
        let targets = vec![
            target(10, "192.168.1.10", "00:10:00:00:00:0A", "access-sw"),
            target(11, "192.168.1.11", "00:10:00:00:00:0B", "access-sw"),
            target(2, "192.168.1.2", "00:1A:2B:00:00:02", "core-sw"),
        ];

        let mut by_name_only = switch_view();
        by_name_only.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("DE:AD:BE:EF:00:99".into()),
            chassis_subtype: Some(4),
            port_id: Some("Gi0/1".into()),
            port_desc: None,
            sys_name: Some("access-sw".into()),
            sys_desc: None,
            management_address: None,
        });
        let snap = correlate(&[by_name_only], &targets, "t");
        let link = &snap.connections[0];
        assert_eq!(link.to_device_id, None);
        assert!(link.to_unresolved_id.is_some());
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);

        let mut by_ip = switch_view();
        by_ip.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("DE:AD:BE:EF:00:99".into()),
            chassis_subtype: Some(4),
            port_id: Some("Gi0/1".into()),
            port_desc: None,
            sys_name: Some("access-sw".into()),
            sys_desc: None,
            management_address: Some("192.168.1.11".into()),
        });
        let snap = correlate(&[by_ip], &targets, "t");
        assert_eq!(
            snap.connections[0].to_device_id,
            Some(11),
            "management IP is strong identity"
        );

        let mut by_mac = switch_view();
        by_mac.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 36,
            chassis_id: Some("00:10:00:00:00:0A".into()),
            chassis_subtype: Some(4),
            port_id: Some("Gi0/1".into()),
            port_desc: None,
            sys_name: Some("access-sw".into()),
            sys_desc: None,
            management_address: None,
        });
        let snap = correlate(&[by_mac], &targets, "t");
        assert_eq!(
            snap.connections[0].to_device_id,
            Some(10),
            "chassis MAC is strong identity"
        );
    }

    #[test]
    fn cdp_device_id_alone_does_not_resolve() {
        let mut core = switch_view();
        core.cdp_neighbors.push(CdpNeighbor {
            if_index: 36,
            device_id: Some("access-sw".into()),
            device_port: Some("GigabitEthernet0/1".into()),
            platform: Some("Cisco IOS".into()),
            address: None,
            native_vlan: Some(10),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        assert_eq!(snap.connections[0].to_device_id, None);
        assert!(snap.connections[0].to_unresolved_id.is_some());
        assert_eq!(snap.unknown_nodes[0].sys_name.as_deref(), Some("access-sw"));
    }

    #[test]
    fn partial_snmp_without_lldp_or_fdb_produces_no_links() {
        let mut core = DeviceView::new(Ipv4Addr::new(192, 168, 1, 2), Some(2));
        core.sys_name = Some("core-sw".into());
        core.interfaces.insert(
            1,
            Iface {
                index: 1,
                name: Some("vlan1".into()),
                descr: None,
                alias: None,
                mac: None,
                if_type: Some(6),
                admin_status: Some(1),
                oper_status: Some(1),
                speed_mbps: Some(1000),
                poe: None,
            },
        );
        let snap = correlate(&[core], &site_targets(), "t");
        assert!(snap.connections.is_empty());
        assert!(snap.edge.is_none());
        assert!(snap.logical_nodes.is_empty());
    }

    #[test]
    fn fdb_does_not_invent_an_endpoint_port() {
        let mut core = switch_view();
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        let snap = correlate(&[core], &site_targets(), "t");
        let link = snap
            .connections
            .iter()
            .find(|c| c.to_device_id == Some(5))
            .unwrap();
        assert_eq!(link.from_port.as_deref(), Some("Port 7"));
        assert_eq!(link.to_port, None);
    }

    #[test]
    fn default_gateway_correlation_mints_internet_node() {
        let mut core = switch_view();
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        let hint = EdgeHint {
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: Some("00:20:AA:00:00:01".into()),
        };
        let snap = correlate_with_edge(&[core], &site_targets(), "t", Some(&hint));
        let edge = snap.edge.as_ref().expect("edge");
        assert_eq!(edge.gateway_device_id, Some(1));
        assert_eq!(edge.internet.id, crate::display::INTERNET_NODE_ID);
        assert!(!edge.internet.physical);
        assert_eq!(edge.internet.label, "Internet");
        assert_eq!(edge.via_unresolved_id, None);
        assert_eq!(edge.via_device_id, None);
        assert_eq!(edge.uplink.kind, "wan");
        assert_eq!(edge.uplink.protocol, "default-route");
        assert_eq!(
            edge.uplink.from_logical_id.as_deref(),
            Some(INTERNET_NODE_ID)
        );
        assert_eq!(edge.uplink.to_device_id, Some(1));
        assert_eq!(edge.uplink.from_port, None);
        assert_eq!(edge.uplink.to_port, None);
        assert_eq!(snap.logical_nodes.len(), 1);
        assert!(!snap.logical_nodes[0].physical);
        assert!(snap
            .connections
            .iter()
            .any(|c| c.kind == "wan" && c.to_device_id == Some(1)));
    }

    #[test]
    fn unmatched_default_route_does_not_invent_a_gateway() {
        let core = switch_view();
        let hint = EdgeHint {
            gateway_ip: Some("10.255.255.1".into()),
            gateway_mac: Some("DE:AD:00:00:00:01".into()),
        };
        let snap = correlate_with_edge(&[core], &site_targets(), "t", Some(&hint));
        assert!(snap.edge.is_none());
        assert!(snap.logical_nodes.is_empty());
        assert!(!snap.connections.iter().any(|c| c.kind == "wan"));
    }

    #[test]
    fn conflicting_gateway_ip_and_mac_are_refused() {
        let core = switch_view();
        let hint = EdgeHint {
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: Some("00:1A:2B:00:00:02".into()),
        };
        let snap = correlate_with_edge(&[core], &site_targets(), "t", Some(&hint));
        assert!(snap.edge.is_none());
    }

    #[test]
    fn ont_neighbour_is_preserved_between_internet_and_gateway() {
        let mut fw = DeviceView::new(Ipv4Addr::new(192, 168, 1, 1), Some(1));
        fw.sys_name = Some("sonicwall".into());
        fw.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 1,
            chassis_id: Some("AA:00:00:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("gpon0".into()),
            port_desc: None,
            sys_name: Some("ONT-01".into()),
            sys_desc: Some("GPON Optical Network Terminal".into()),
            management_address: None,
        });
        let hint = EdgeHint {
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: Some("00:20:AA:00:00:01".into()),
        };
        let snap = correlate_with_edge(&[fw], &site_targets(), "t", Some(&hint));
        let edge = snap.edge.as_ref().expect("edge");
        assert_eq!(edge.gateway_device_id, Some(1));
        let via = edge.via_unresolved_id.as_deref().expect("ont");
        assert!(via.starts_with("unknown:"));
        assert_eq!(edge.via_device_id, None);
        assert_eq!(edge.uplink.to_unresolved_id.as_deref(), Some(via));
        assert_eq!(edge.uplink.to_device_id, None);
        assert!(snap.unknown_nodes.iter().any(|n| n.id == via));
        assert!(snap
            .unknown_nodes
            .iter()
            .any(|n| n.sys_name.as_deref() == Some("ONT-01")));
    }

    #[test]
    fn known_inventory_ont_is_kept_between_internet_and_gateway() {
        let mut fw = DeviceView::new(Ipv4Addr::new(192, 168, 1, 1), Some(1));
        fw.sys_name = Some("sonicwall".into());
        fw.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 1,
            chassis_id: Some("AA:00:00:00:00:09".into()),
            chassis_subtype: Some(4),
            port_id: Some("X1".into()),
            port_desc: None,
            sys_name: Some("ONT-01".into()),
            sys_desc: Some("GPON Optical Network Terminal".into()),
            management_address: Some("192.168.1.9".into()),
        });
        let mut targets = site_targets();
        targets.push(target(9, "192.168.1.9", "AA:00:00:00:00:09", "ONT-01"));
        let hint = EdgeHint {
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: Some("00:20:AA:00:00:01".into()),
        };
        let snap = correlate_with_edge(&[fw], &targets, "t", Some(&hint));
        let edge = snap.edge.as_ref().expect("edge");
        assert_eq!(edge.gateway_device_id, Some(1));
        assert_eq!(edge.via_device_id, Some(9));
        assert_eq!(edge.via_unresolved_id, None);
        assert_eq!(
            edge.uplink.from_logical_id.as_deref(),
            Some(INTERNET_NODE_ID)
        );
        assert_eq!(edge.uplink.to_device_id, Some(9));
        assert_eq!(edge.uplink.to_unresolved_id, None);
        assert_eq!(edge.uplink.kind, "wan");
        assert!(snap
            .connections
            .iter()
            .any(|c| c.kind == "wan" && c.to_device_id == Some(9)));
        assert!(!snap
            .connections
            .iter()
            .any(|c| c.kind == "wan" && c.to_device_id == Some(1)));
        // The ordinary LLDP neighbour link still connects gateway ↔ ONT, so the
        // hierarchy is Internet → ONT → gateway rather than Internet → gateway.
        assert!(snap.connections.iter().any(|c| {
            c.kind != "wan"
                && c.protocol == "lldp"
                && ((c.from_device_id == Some(1) && c.to_device_id == Some(9))
                    || (c.from_device_id == Some(9) && c.to_device_id == Some(1)))
        }));
        assert!(snap.unknown_nodes.is_empty());
    }

    #[test]
    fn internet_node_is_never_an_inventory_device() {
        let core = switch_view();
        let hint = EdgeHint {
            gateway_ip: Some("192.168.1.1".into()),
            gateway_mac: None,
        };
        let snap = correlate_with_edge(&[core], &site_targets(), "t", Some(&hint));
        let internet = snap.logical_nodes.iter().find(|n| n.kind == "internet");
        assert!(internet.is_some());
        assert!(!internet.unwrap().physical);
        assert!(snap
            .connections
            .iter()
            .filter(|c| c.kind == "wan")
            .all(|c| c.from_device_id.is_none() && c.from_logical_id.is_some()));
        for target in site_targets() {
            assert_ne!(target.device_id, Some(-1));
        }
    }
}
