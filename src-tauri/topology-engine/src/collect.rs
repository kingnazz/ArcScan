//! Walk the topology MIBs on one device and produce a [`DeviceView`].
//!
//! Partial support is success: a switch that answers IF-MIB but not LLDP still
//! contributes interfaces, FDB and ARP. One failed walk is recorded and the
//! rest continue.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use super::ber::{Oid, SnmpValue, VarBind};
use super::error::TopologyError;
use super::model::PoeInfo;
use super::snmp::SnmpSession;

pub const SYS_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
pub const SYS_OBJECT_ID: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 2, 0];
pub const SYS_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 5, 0];

pub const IF_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
pub const IF_TYPE: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 3];
pub const IF_SPEED: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 5];
pub const IF_PHYS_ADDRESS: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 6];
pub const IF_ADMIN_STATUS: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 7];
pub const IF_OPER_STATUS: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8];
pub const IF_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1];
pub const IF_HIGH_SPEED: &[u32] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 15];
pub const IF_ALIAS: &[u32] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 18];

pub const DOT1D_BASE_BRIDGE_ADDRESS: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 1, 1, 0];
pub const DOT1D_BASE_PORT_IF_INDEX: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 1, 4, 1, 2];
pub const DOT1D_TP_FDB_ADDRESS: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 4, 3, 1, 1];
pub const DOT1D_TP_FDB_PORT: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 4, 3, 1, 2];
pub const DOT1D_TP_FDB_STATUS: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 4, 3, 1, 3];

pub const DOT1Q_TP_FDB_PORT: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 7, 1, 2, 2, 1, 2];
pub const DOT1Q_VLAN_CURRENT_EGRESS: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 7, 1, 4, 2, 1, 4];
pub const DOT1Q_VLAN_CURRENT_UNTAGGED: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 7, 1, 4, 2, 1, 5];
pub const DOT1Q_PVID: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 7, 1, 4, 5, 1, 1];

pub const IP_NET_TO_MEDIA_PHYS: &[u32] = &[1, 3, 6, 1, 2, 1, 4, 22, 1, 2];
pub const IP_NET_TO_PHYSICAL_PHYS: &[u32] = &[1, 3, 6, 1, 2, 1, 4, 35, 1, 4];

pub const LLDP_LOC_CHASSIS_SUBTYPE: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 1, 0];
pub const LLDP_LOC_CHASSIS_ID: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 2, 0];
pub const LLDP_LOC_SYS_NAME: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 3, 0];
pub const LLDP_LOC_PORT_ID: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 7, 1, 3];
pub const LLDP_LOC_PORT_DESC: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 7, 1, 4];
pub const LLDP_REM_CHASSIS_SUBTYPE: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 4];
pub const LLDP_REM_CHASSIS_ID: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 5];
pub const LLDP_REM_PORT_ID: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 7];
pub const LLDP_REM_PORT_DESC: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 8];
pub const LLDP_REM_SYS_NAME: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 9];
pub const LLDP_REM_SYS_DESC: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 10];
pub const LLDP_REM_MAN_ADDR: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 2, 1, 4];

pub const CDP_CACHE_ADDRESS: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 23, 1, 2, 1, 1, 4];
pub const CDP_CACHE_DEVICE_ID: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 23, 1, 2, 1, 1, 6];
pub const CDP_CACHE_DEVICE_PORT: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 23, 1, 2, 1, 1, 7];
pub const CDP_CACHE_PLATFORM: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 23, 1, 2, 1, 1, 8];
pub const CDP_CACHE_NATIVE_VLAN: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 23, 1, 2, 1, 1, 11];

pub const PETH_PSE_DETECTION: &[u32] = &[1, 3, 6, 1, 2, 1, 105, 1, 1, 1, 6];
pub const CPE_EXT_PWR_ALLOCATED: &[u32] = &[1, 3, 6, 1, 4, 1, 9, 9, 402, 1, 2, 1, 7];

pub const ENT_PHYSICAL_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 2];
pub const ENT_PHYSICAL_MFG: &[u32] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 12];
pub const ENT_PHYSICAL_MODEL: &[u32] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 13];

#[derive(Debug, Clone, Default)]
pub struct Iface {
    pub index: u32,
    pub descr: Option<String>,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub mac: Option<String>,
    pub if_type: Option<u32>,
    pub admin_status: Option<u8>,
    pub oper_status: Option<u8>,
    pub speed_mbps: Option<u64>,
    pub poe: Option<PoeInfo>,
}

impl Iface {
    pub fn display_name(&self) -> String {
        crate::display::first_printable([
            self.name.as_deref(),
            self.alias.as_deref(),
            self.descr.as_deref(),
        ])
        .unwrap_or_else(|| self.index.to_string())
    }

    pub fn is_up(&self) -> bool {
        self.oper_status == Some(1)
    }
}

#[derive(Debug, Clone, Default)]
pub struct LldpNeighbor {
    pub local_port_num: u32,
    pub chassis_id: Option<String>,
    pub chassis_subtype: Option<i64>,
    pub port_id: Option<String>,
    pub port_desc: Option<String>,
    pub sys_name: Option<String>,
    pub sys_desc: Option<String>,
    pub management_address: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CdpNeighbor {
    pub if_index: u32,
    pub device_id: Option<String>,
    pub device_port: Option<String>,
    pub platform: Option<String>,
    pub address: Option<String>,
    pub native_vlan: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct FdbEntry {
    pub mac: String,
    pub if_index: u32,
    pub vlan: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct ArpEntry {
    pub ip: String,
    pub mac: String,
    pub if_index: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct EntityInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub descr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceView {
    pub target_ip: Ipv4Addr,
    pub inventory_hint: Option<i64>,
    pub sys_name: Option<String>,
    pub sys_descr: Option<String>,
    pub sys_object_id: Option<String>,
    pub bridge_address: Option<String>,
    pub chassis_id: Option<String>,
    pub loc_sys_name: Option<String>,
    pub interfaces: BTreeMap<u32, Iface>,
    pub lldp_local_ports: BTreeMap<u32, String>,
    pub lldp_neighbors: Vec<LldpNeighbor>,
    pub cdp_neighbors: Vec<CdpNeighbor>,
    pub fdb: Vec<FdbEntry>,
    pub arp: Vec<ArpEntry>,
    /// ifIndex → PVID
    pub pvid: BTreeMap<u32, u16>,
    /// ifIndex → tagged VLAN ids
    pub tagged: BTreeMap<u32, BTreeSet<u16>>,
    pub untagged: BTreeMap<u32, BTreeSet<u16>>,
    pub entity: EntityInfo,
    pub mibs_present: Vec<String>,
    pub notes: Vec<String>,
    /// Bridge port number → ifIndex, from BRIDGE-MIB.
    pub bridge_port_if: BTreeMap<u32, u32>,
}

impl DeviceView {
    pub fn new(ip: Ipv4Addr, inventory_hint: Option<i64>) -> Self {
        Self {
            target_ip: ip,
            inventory_hint,
            sys_name: None,
            sys_descr: None,
            sys_object_id: None,
            bridge_address: None,
            chassis_id: None,
            loc_sys_name: None,
            interfaces: BTreeMap::new(),
            lldp_local_ports: BTreeMap::new(),
            lldp_neighbors: Vec::new(),
            cdp_neighbors: Vec::new(),
            fdb: Vec::new(),
            arp: Vec::new(),
            pvid: BTreeMap::new(),
            tagged: BTreeMap::new(),
            untagged: BTreeMap::new(),
            entity: EntityInfo::default(),
            mibs_present: Vec::new(),
            notes: Vec::new(),
            bridge_port_if: BTreeMap::new(),
        }
    }

    pub fn own_macs(&self) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        if let Some(mac) = &self.bridge_address {
            set.insert(mac.clone());
        }
        if let Some(mac) = &self.chassis_id {
            if looks_like_mac(mac) {
                set.insert(normalize_mac(mac).unwrap_or_else(|| mac.clone()));
            }
        }
        for iface in self.interfaces.values() {
            if let Some(mac) = &iface.mac {
                set.insert(mac.clone());
            }
        }
        set
    }

    pub fn iface(&self, if_index: u32) -> Option<&Iface> {
        self.interfaces.get(&if_index)
    }

    pub fn port_name(&self, if_index: u32) -> String {
        let resolved = self.resolve_if_index(if_index);
        self.iface(resolved)
            .map(Iface::display_name)
            .unwrap_or_else(|| resolved.to_string())
    }

    /// Map an LLDP locPortNum or bridge port onto IF-MIB ifIndex when the
    /// agent numbers them differently.
    pub fn resolve_if_index(&self, port: u32) -> u32 {
        if self.interfaces.contains_key(&port) {
            return port;
        }
        if let Some(idx) = self.bridge_port_if.get(&port) {
            return *idx;
        }
        if let Some(name) = self.lldp_local_ports.get(&port) {
            if let Some((idx, _)) = self.interfaces.iter().find(|(_, iface)| {
                iface.name.as_deref() == Some(name.as_str())
                    || iface.alias.as_deref() == Some(name.as_str())
                    || iface.descr.as_deref() == Some(name.as_str())
            }) {
                return *idx;
            }
        }
        port
    }

    pub fn vlan_for_port(&self, if_index: u32) -> (Option<String>, Option<u16>, Vec<u16>) {
        let native = self.pvid.get(&if_index).copied();
        let tagged: Vec<u16> = self
            .tagged
            .get(&if_index)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        if tagged.len() > 1
            || (tagged.len() == 1 && native.is_some() && !tagged.contains(&native.unwrap()))
        {
            return (Some("trunk".into()), native, tagged);
        }
        if let Some(vid) = native {
            return (Some(vid.to_string()), Some(vid), Vec::new());
        }
        (None, None, tagged)
    }

    pub fn is_access_port(&self, if_index: u32) -> bool {
        let (_, _, tagged) = self.vlan_for_port(if_index);
        tagged.is_empty()
    }
}

pub async fn collect_device(
    session: &dyn SnmpSession,
    ip: Ipv4Addr,
    inventory_hint: Option<i64>,
) -> Result<DeviceView, TopologyError> {
    if let Some(err) = session.interrupted() {
        return Err(err);
    }
    let mut view = DeviceView::new(ip, inventory_hint);

    match session
        .get(&[
            Oid::from_slice(SYS_NAME),
            Oid::from_slice(SYS_DESCR),
            Oid::from_slice(SYS_OBJECT_ID),
            Oid::from_slice(DOT1D_BASE_BRIDGE_ADDRESS),
            Oid::from_slice(LLDP_LOC_CHASSIS_ID),
            Oid::from_slice(LLDP_LOC_SYS_NAME),
            Oid::from_slice(LLDP_LOC_CHASSIS_SUBTYPE),
        ])
        .await
    {
        Ok(binds) => {
            for bind in binds {
                if bind.oid.0 == SYS_NAME {
                    view.sys_name = bind.value.as_utf8();
                } else if bind.oid.0 == SYS_DESCR {
                    view.sys_descr = bind.value.as_utf8();
                } else if bind.oid.0 == SYS_OBJECT_ID {
                    view.sys_object_id = bind.value.as_utf8();
                } else if bind.oid.0 == DOT1D_BASE_BRIDGE_ADDRESS {
                    view.bridge_address = mac_from_value(&bind.value);
                } else if bind.oid.0 == LLDP_LOC_CHASSIS_ID {
                    view.chassis_id = chassis_from_value(&bind.value);
                } else if bind.oid.0 == LLDP_LOC_SYS_NAME {
                    view.loc_sys_name = bind.value.as_utf8();
                }
            }
            view.mibs_present.push("SNMPv2-MIB".into());
        }
        Err(err) => return Err(err),
    }

    walk_ok(session, IF_DESCR, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_DESCR) {
            assign_iface_label(view, "ifDescr", idx, &bind.value, |iface| &mut iface.descr);
        }
    })
    .await?;
    if !view.interfaces.is_empty() {
        view.mibs_present.push("IF-MIB".into());
    }
    walk_ok(session, IF_NAME, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_NAME) {
            assign_iface_label(view, "ifName", idx, &bind.value, |iface| &mut iface.name);
        }
    })
    .await?;
    walk_ok(session, IF_ALIAS, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_ALIAS) {
            assign_iface_label(view, "ifAlias", idx, &bind.value, |iface| &mut iface.alias);
        }
    })
    .await?;
    walk_ok(session, IF_TYPE, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_TYPE) {
            view.interfaces.entry(idx).or_default().if_type = bind.value.as_u64().map(|v| v as u32);
        }
    })
    .await?;
    walk_ok(session, IF_PHYS_ADDRESS, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_PHYS_ADDRESS) {
            view.interfaces.entry(idx).or_default().mac = mac_from_value(&bind.value);
        }
    })
    .await?;
    walk_ok(session, IF_OPER_STATUS, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_OPER_STATUS) {
            view.interfaces.entry(idx).or_default().oper_status =
                bind.value.as_u64().map(|v| v as u8);
        }
    })
    .await?;
    walk_ok(session, IF_ADMIN_STATUS, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_ADMIN_STATUS) {
            view.interfaces.entry(idx).or_default().admin_status =
                bind.value.as_u64().map(|v| v as u8);
        }
    })
    .await?;
    walk_ok(session, IF_HIGH_SPEED, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_HIGH_SPEED) {
            if let Some(mbps) = bind.value.as_u64() {
                if mbps > 0 {
                    view.interfaces.entry(idx).or_default().speed_mbps = Some(mbps);
                }
            }
        }
    })
    .await?;
    walk_ok(session, IF_SPEED, &mut view, |view, bind| {
        if let Some(idx) = last_index(&bind.oid, IF_SPEED) {
            let iface = view.interfaces.entry(idx).or_default();
            if iface.speed_mbps.is_none() {
                if let Some(bps) = bind.value.as_u64() {
                    if bps > 0 {
                        iface.speed_mbps = Some((bps / 1_000_000).max(1));
                    }
                }
            }
        }
    })
    .await?;

    // Bridge port → ifIndex map, used by FDB and PVID.
    let mut bridge_to_if = BTreeMap::new();
    let binds = walk_table(session, DOT1D_BASE_PORT_IF_INDEX).await?;
    for bind in binds {
        if let (Some(port), Some(if_index)) = (
            last_index(&bind.oid, DOT1D_BASE_PORT_IF_INDEX),
            bind.value.as_u64(),
        ) {
            bridge_to_if.insert(port, if_index as u32);
        }
    }
    if !bridge_to_if.is_empty() {
        view.mibs_present.push("BRIDGE-MIB".into());
        view.bridge_port_if = bridge_to_if.clone();
    }

    collect_fdb_bridge(session, &mut view, &bridge_to_if).await?;
    collect_fdb_qbridge(session, &mut view, &bridge_to_if).await?;
    collect_vlans(session, &mut view, &bridge_to_if).await?;
    collect_arp(session, &mut view).await?;
    collect_lldp(session, &mut view).await?;
    collect_cdp(session, &mut view).await?;
    collect_poe(session, &mut view).await?;
    collect_entity(session, &mut view).await?;

    view.mibs_present.sort();
    view.mibs_present.dedup();
    Ok(view)
}

async fn walk_table(
    session: &dyn SnmpSession,
    root: &[u32],
) -> Result<Vec<VarBind>, TopologyError> {
    if let Some(err) = session.interrupted() {
        return Err(err);
    }
    match session.walk(&Oid::from_slice(root)).await {
        Ok(binds) => Ok(binds),
        Err(TopologyError::Cancelled) => Err(TopologyError::Cancelled),
        Err(err) => {
            if let Some(interrupt) = session.interrupted() {
                Err(interrupt)
            } else {
                let _ = err;
                Ok(Vec::new())
            }
        }
    }
}

async fn walk_ok<F>(
    session: &dyn SnmpSession,
    root: &[u32],
    view: &mut DeviceView,
    mut each: F,
) -> Result<(), TopologyError>
where
    F: FnMut(&mut DeviceView, VarBind),
{
    match walk_table(session, root).await {
        Ok(binds) => {
            for bind in binds {
                each(view, bind);
            }
            Ok(())
        }
        Err(TopologyError::Cancelled) => Err(TopologyError::Cancelled),
        Err(err) => {
            if session.interrupted().is_some() {
                Err(err)
            } else {
                view.notes.push(format!(
                    "Partial SNMP support: {} walk failed ({})",
                    Oid::from_slice(root),
                    err
                ));
                Ok(())
            }
        }
    }
}

async fn collect_fdb_bridge(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
    bridge_to_if: &BTreeMap<u32, u32>,
) -> Result<(), TopologyError> {
    let mut ports: BTreeMap<String, u32> = BTreeMap::new();
    let binds = walk_table(session, DOT1D_TP_FDB_PORT).await?;
    for bind in binds {
        if let Some(mac) = mac_from_oid_suffix(&bind.oid, DOT1D_TP_FDB_PORT) {
            if let Some(port) = bind.value.as_u64() {
                ports.insert(mac, port as u32);
            }
        }
    }
    let mut status: BTreeMap<String, i64> = BTreeMap::new();
    let binds = walk_table(session, DOT1D_TP_FDB_STATUS).await?;
    for bind in binds {
        if let Some(mac) = mac_from_oid_suffix(&bind.oid, DOT1D_TP_FDB_STATUS) {
            if let Some(st) = bind.value.as_i64() {
                status.insert(mac, st);
            }
        }
    }
    for (mac, port) in ports {
        // 3 = learned, 5 = remaining in some agents. Skip self/invalid/mgmt.
        if matches!(status.get(&mac).copied(), Some(1) | Some(2) | Some(4)) {
            continue;
        }
        let if_index = *bridge_to_if.get(&port).unwrap_or(&port);
        view.fdb.push(FdbEntry {
            mac,
            if_index,
            vlan: None,
        });
    }
    Ok(())
}

async fn collect_fdb_qbridge(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
    bridge_to_if: &BTreeMap<u32, u32>,
) -> Result<(), TopologyError> {
    let binds = walk_table(session, DOT1Q_TP_FDB_PORT).await?;
    if !binds.is_empty() {
        view.mibs_present.push("Q-BRIDGE-MIB".into());
    }
    for bind in binds {
        if let Some((vlan, mac)) = vlan_mac_from_oid(&bind.oid, DOT1Q_TP_FDB_PORT) {
            if let Some(port) = bind.value.as_u64() {
                let if_index = *bridge_to_if.get(&(port as u32)).unwrap_or(&(port as u32));
                view.fdb.push(FdbEntry {
                    mac,
                    if_index,
                    vlan: Some(vlan),
                });
            }
        }
    }
    Ok(())
}

async fn collect_vlans(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
    bridge_to_if: &BTreeMap<u32, u32>,
) -> Result<(), TopologyError> {
    walk_ok(session, DOT1Q_PVID, view, |view, bind| {
        if let Some(port) = last_index(&bind.oid, DOT1Q_PVID) {
            if let Some(vid) = bind.value.as_u64() {
                let if_index = *bridge_to_if.get(&port).unwrap_or(&port);
                view.pvid.insert(if_index, vid as u16);
            }
        }
    })
    .await?;

    let binds = walk_table(session, DOT1Q_VLAN_CURRENT_EGRESS).await?;
    for bind in binds {
        if let (Some(vid), Some(bytes)) = (
            last_index(&bind.oid, DOT1Q_VLAN_CURRENT_EGRESS),
            bind.value.as_bytes(),
        ) {
            for port in ports_from_bitstring(bytes) {
                let if_index = *bridge_to_if.get(&port).unwrap_or(&port);
                view.tagged.entry(if_index).or_default().insert(vid as u16);
            }
        }
    }
    let binds = walk_table(session, DOT1Q_VLAN_CURRENT_UNTAGGED).await?;
    for bind in binds {
        if let (Some(vid), Some(bytes)) = (
            last_index(&bind.oid, DOT1Q_VLAN_CURRENT_UNTAGGED),
            bind.value.as_bytes(),
        ) {
            for port in ports_from_bitstring(bytes) {
                let if_index = *bridge_to_if.get(&port).unwrap_or(&port);
                view.untagged
                    .entry(if_index)
                    .or_default()
                    .insert(vid as u16);
                view.pvid.entry(if_index).or_insert(vid as u16);
                if let Some(set) = view.tagged.get_mut(&if_index) {
                    set.remove(&(vid as u16));
                }
            }
        }
    }
    Ok(())
}

async fn collect_arp(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
) -> Result<(), TopologyError> {
    let binds = walk_table(session, IP_NET_TO_MEDIA_PHYS).await?;
    if !binds.is_empty() {
        view.mibs_present.push("IP-MIB".into());
    }
    for bind in binds {
        if let Some((if_index, ip)) = arp_index(&bind.oid, IP_NET_TO_MEDIA_PHYS) {
            if let Some(mac) = mac_from_value(&bind.value) {
                view.arp.push(ArpEntry {
                    ip,
                    mac,
                    if_index: Some(if_index),
                });
            }
        }
    }
    if view.arp.is_empty() {
        let binds = walk_table(session, IP_NET_TO_PHYSICAL_PHYS).await?;
        for bind in binds {
            if let Some(mac) = mac_from_value(&bind.value) {
                if let Some(ip) = ip_from_neighbor_oid(&bind.oid) {
                    view.arp.push(ArpEntry {
                        ip,
                        mac,
                        if_index: None,
                    });
                }
            }
        }
    }
    Ok(())
}

async fn collect_lldp(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
) -> Result<(), TopologyError> {
    walk_ok(session, LLDP_LOC_PORT_ID, view, |view, bind| {
        if let Some(port) = last_index(&bind.oid, LLDP_LOC_PORT_ID) {
            if let Some(name) = mac_from_value(&bind.value).or_else(|| bind.value.as_utf8()) {
                view.lldp_local_ports.insert(port, name);
            }
        }
    })
    .await?;
    walk_ok(session, LLDP_LOC_PORT_DESC, view, |view, bind| {
        if let Some(port) = last_index(&bind.oid, LLDP_LOC_PORT_DESC) {
            if let Some(desc) = bind.value.as_utf8() {
                view.lldp_local_ports.entry(port).or_insert(desc);
            }
        }
    })
    .await?;

    let mut neighbors: BTreeMap<(u32, u32), LldpNeighbor> = BTreeMap::new();

    let binds = walk_table(session, LLDP_REM_CHASSIS_ID).await?;
    if !binds.is_empty() {
        view.mibs_present.push("LLDP-MIB".into());
    }
    for bind in binds {
        if let Some((_, local, rem)) = lldp_rem_index(&bind.oid, LLDP_REM_CHASSIS_ID) {
            let entry = neighbors
                .entry((local, rem))
                .or_insert_with(|| LldpNeighbor {
                    local_port_num: local,
                    ..LldpNeighbor::default()
                });
            entry.chassis_id = chassis_from_value(&bind.value);
        }
    }
    fill_lldp(session, LLDP_REM_CHASSIS_SUBTYPE, &mut neighbors, |n, v| {
        n.chassis_subtype = v.as_i64();
    })
    .await?;
    fill_lldp(session, LLDP_REM_PORT_ID, &mut neighbors, |n, v| {
        n.port_id = mac_from_value(v).or_else(|| v.as_utf8());
    })
    .await?;
    fill_lldp(session, LLDP_REM_PORT_DESC, &mut neighbors, |n, v| {
        n.port_desc = v.as_utf8();
    })
    .await?;
    fill_lldp(session, LLDP_REM_SYS_NAME, &mut neighbors, |n, v| {
        n.sys_name = v.as_utf8();
    })
    .await?;
    fill_lldp(session, LLDP_REM_SYS_DESC, &mut neighbors, |n, v| {
        n.sys_desc = v.as_utf8();
    })
    .await?;
    let binds = walk_table(session, LLDP_REM_MAN_ADDR).await?;
    for bind in binds {
        if let Some((_, local, rem)) = lldp_rem_index(&bind.oid, LLDP_REM_MAN_ADDR) {
            let entry = neighbors
                .entry((local, rem))
                .or_insert_with(|| LldpNeighbor {
                    local_port_num: local,
                    ..LldpNeighbor::default()
                });
            if entry.management_address.is_none() {
                entry.management_address =
                    bind.value.as_ip().map(|ip| ip.to_string()).or_else(|| {
                        bind.value.as_bytes().and_then(|b| {
                            if b.len() == 4 {
                                Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string())
                            } else {
                                bind.value.as_utf8()
                            }
                        })
                    });
            }
        }
    }
    view.lldp_neighbors = neighbors.into_values().collect();
    Ok(())
}

async fn fill_lldp<F>(
    session: &dyn SnmpSession,
    prefix: &[u32],
    neighbors: &mut BTreeMap<(u32, u32), LldpNeighbor>,
    mut set: F,
) -> Result<(), TopologyError>
where
    F: FnMut(&mut LldpNeighbor, &SnmpValue),
{
    let binds = walk_table(session, prefix).await?;
    for bind in binds {
        if let Some((_, local, rem)) = lldp_rem_index(&bind.oid, prefix) {
            let entry = neighbors
                .entry((local, rem))
                .or_insert_with(|| LldpNeighbor {
                    local_port_num: local,
                    ..LldpNeighbor::default()
                });
            set(entry, &bind.value);
        }
    }
    Ok(())
}

async fn collect_cdp(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
) -> Result<(), TopologyError> {
    let mut neighbors: BTreeMap<(u32, u32), CdpNeighbor> = BTreeMap::new();
    let binds = walk_table(session, CDP_CACHE_DEVICE_ID).await?;
    if !binds.is_empty() {
        view.mibs_present.push("CISCO-CDP-MIB".into());
    }
    for bind in binds {
        if let Some((if_index, dev)) = two_index(&bind.oid, CDP_CACHE_DEVICE_ID) {
            let entry = neighbors
                .entry((if_index, dev))
                .or_insert_with(|| CdpNeighbor {
                    if_index,
                    ..CdpNeighbor::default()
                });
            entry.device_id = bind.value.as_utf8();
        }
    }
    let binds = walk_table(session, CDP_CACHE_DEVICE_PORT).await?;
    for bind in binds {
        if let Some((if_index, dev)) = two_index(&bind.oid, CDP_CACHE_DEVICE_PORT) {
            neighbors.entry((if_index, dev)).or_default().device_port = bind.value.as_utf8();
            neighbors.entry((if_index, dev)).or_default().if_index = if_index;
        }
    }
    let binds = walk_table(session, CDP_CACHE_PLATFORM).await?;
    for bind in binds {
        if let Some((if_index, dev)) = two_index(&bind.oid, CDP_CACHE_PLATFORM) {
            neighbors.entry((if_index, dev)).or_default().platform = bind.value.as_utf8();
            neighbors.entry((if_index, dev)).or_default().if_index = if_index;
        }
    }
    let binds = walk_table(session, CDP_CACHE_ADDRESS).await?;
    for bind in binds {
        if let Some((if_index, dev)) = two_index(&bind.oid, CDP_CACHE_ADDRESS) {
            neighbors.entry((if_index, dev)).or_default().address = bind
                .value
                .as_ip()
                .map(|ip| ip.to_string())
                .or_else(|| bind.value.as_utf8());
            neighbors.entry((if_index, dev)).or_default().if_index = if_index;
        }
    }
    let binds = walk_table(session, CDP_CACHE_NATIVE_VLAN).await?;
    for bind in binds {
        if let Some((if_index, dev)) = two_index(&bind.oid, CDP_CACHE_NATIVE_VLAN) {
            neighbors.entry((if_index, dev)).or_default().native_vlan =
                bind.value.as_u64().map(|v| v as u16);
            neighbors.entry((if_index, dev)).or_default().if_index = if_index;
        }
    }
    view.cdp_neighbors = neighbors.into_values().collect();
    Ok(())
}

async fn collect_poe(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
) -> Result<(), TopologyError> {
    let mut detection: BTreeMap<u32, i64> = BTreeMap::new();
    let binds = walk_table(session, PETH_PSE_DETECTION).await?;
    if !binds.is_empty() {
        view.mibs_present.push("POWER-ETHERNET-MIB".into());
    }
    for bind in binds {
        if let Some(if_index) = last_index(&bind.oid, PETH_PSE_DETECTION) {
            if let Some(st) = bind.value.as_i64() {
                detection.insert(if_index, st);
            }
        }
    }
    let mut watts: BTreeMap<u32, f64> = BTreeMap::new();
    let binds = walk_table(session, CPE_EXT_PWR_ALLOCATED).await?;
    for bind in binds {
        if let Some(if_index) = last_index(&bind.oid, CPE_EXT_PWR_ALLOCATED) {
            if let Some(mw) = bind.value.as_u64() {
                watts.insert(if_index, mw as f64 / 1000.0);
            }
        }
    }
    for (if_index, status) in detection {
        let enabled = status == 3;
        if enabled || watts.contains_key(&if_index) {
            let iface = view.interfaces.entry(if_index).or_default();
            iface.index = if_index;
            iface.poe = Some(PoeInfo {
                enabled,
                watts: watts.get(&if_index).copied(),
            });
        }
    }
    Ok(())
}

async fn collect_entity(
    session: &dyn SnmpSession,
    view: &mut DeviceView,
) -> Result<(), TopologyError> {
    let binds = walk_table(session, ENT_PHYSICAL_MODEL).await?;
    if !binds.is_empty() {
        view.mibs_present.push("ENTITY-MIB".into());
        view.entity.model = binds.iter().find_map(|b| b.value.as_utf8());
    }
    let binds = walk_table(session, ENT_PHYSICAL_MFG).await?;
    view.entity.manufacturer = binds.iter().find_map(|b| b.value.as_utf8());
    let binds = walk_table(session, ENT_PHYSICAL_DESCR).await?;
    view.entity.descr = binds.iter().find_map(|b| b.value.as_utf8());
    Ok(())
}

fn assign_iface_label<F>(view: &mut DeviceView, field: &str, idx: u32, value: &SnmpValue, slot: F)
where
    F: FnOnce(&mut Iface) -> &mut Option<String>,
{
    let iface = view.interfaces.entry(idx).or_default();
    iface.index = idx;
    if let Some(label) = value.as_utf8() {
        *slot(iface) = Some(label);
        return;
    }
    if let Some(bytes) = value.as_bytes() {
        if crate::display::is_nonempty_octets(bytes) {
            view.notes
                .push(crate::display::rejected_octets_note(field, idx, bytes));
        }
    }
}

fn last_index(oid: &Oid, prefix: &[u32]) -> Option<u32> {
    let suffix = oid.suffix_after(prefix)?;
    suffix.last().copied()
}

fn two_index(oid: &Oid, prefix: &[u32]) -> Option<(u32, u32)> {
    let suffix = oid.suffix_after(prefix)?;
    if suffix.len() >= 2 {
        Some((suffix[0], suffix[1]))
    } else {
        None
    }
}

fn lldp_rem_index(oid: &Oid, prefix: &[u32]) -> Option<(u32, u32, u32)> {
    let suffix = oid.suffix_after(prefix)?;
    if suffix.len() >= 3 {
        Some((suffix[0], suffix[1], suffix[2]))
    } else if suffix.len() == 2 {
        Some((0, suffix[0], suffix[1]))
    } else {
        None
    }
}

fn mac_from_oid_suffix(oid: &Oid, prefix: &[u32]) -> Option<String> {
    let suffix = oid.suffix_after(prefix)?;
    if suffix.len() >= 6 {
        let mac = &suffix[suffix.len() - 6..];
        return Some(format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ));
    }
    None
}

fn vlan_mac_from_oid(oid: &Oid, prefix: &[u32]) -> Option<(u16, String)> {
    let suffix = oid.suffix_after(prefix)?;
    if suffix.len() >= 7 {
        let vlan = suffix[0] as u16;
        let mac = &suffix[suffix.len() - 6..];
        let formatted = format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        return Some((vlan, formatted));
    }
    None
}

fn arp_index(oid: &Oid, prefix: &[u32]) -> Option<(u32, String)> {
    let suffix = oid.suffix_after(prefix)?;
    if suffix.len() >= 5 {
        let if_index = suffix[0];
        let ip = Ipv4Addr::new(
            suffix[suffix.len() - 4] as u8,
            suffix[suffix.len() - 3] as u8,
            suffix[suffix.len() - 2] as u8,
            suffix[suffix.len() - 1] as u8,
        );
        Some((if_index, ip.to_string()))
    } else {
        None
    }
}

fn ip_from_neighbor_oid(oid: &Oid) -> Option<String> {
    // ipNetToPhysicalPhysAddress: ifIndex.family.addrType.len.octets...
    let arcs = &oid.0;
    if arcs.len() >= 4 {
        let last4 = &arcs[arcs.len() - 4..];
        if last4.iter().all(|n| *n <= 255) {
            return Some(
                Ipv4Addr::new(
                    last4[0] as u8,
                    last4[1] as u8,
                    last4[2] as u8,
                    last4[3] as u8,
                )
                .to_string(),
            );
        }
    }
    None
}

fn ports_from_bitstring(bytes: &[u8]) -> Vec<u32> {
    let mut ports = Vec::new();
    for (octet_i, octet) in bytes.iter().enumerate() {
        for bit in 0..8 {
            if octet & (1 << (7 - bit)) != 0 {
                ports.push((octet_i as u32) * 8 + bit as u32 + 1);
            }
        }
    }
    ports
}

pub fn mac_from_value(value: &SnmpValue) -> Option<String> {
    if let Some(bytes) = value.as_bytes() {
        if bytes.len() == 6 {
            return normalize_mac(&format!(
                "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
            ));
        }
        if let Ok(s) = std::str::from_utf8(bytes) {
            return normalize_mac(s);
        }
    }
    value.as_utf8().and_then(|s| normalize_mac(&s))
}

fn chassis_from_value(value: &SnmpValue) -> Option<String> {
    mac_from_value(value).or_else(|| value.as_utf8())
}

pub fn normalize_mac(raw: &str) -> Option<String> {
    let hex: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase();
    if hex.len() != 12 {
        return None;
    }
    let joined = hex
        .as_bytes()
        .chunks(2)
        .map(|p| std::str::from_utf8(p).unwrap_or("00"))
        .collect::<Vec<_>>()
        .join(":");
    if joined == "00:00:00:00:00:00" || joined == "FF:FF:FF:FF:FF:FF" {
        return None;
    }
    Some(joined)
}

pub fn looks_like_mac(s: &str) -> bool {
    normalize_mac(s).is_some()
}

pub fn is_unicast_mac(mac: &str) -> bool {
    let Some(norm) = normalize_mac(mac) else {
        return false;
    };
    let first = u8::from_str_radix(&norm[0..2], 16).unwrap_or(1);
    first & 0x01 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TopologyError;
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;

    #[test]
    fn bitstring_ports() {
        // Port 1 = 0x80, port 8 = 0x01, port 9 = second octet 0x80.
        assert_eq!(ports_from_bitstring(&[0x80]), vec![1]);
        assert_eq!(ports_from_bitstring(&[0x01, 0x80]), vec![8, 9]);
    }

    #[test]
    fn mac_from_six_bytes() {
        let v = SnmpValue::OctetString(vec![0x00, 0x1A, 0x2B, 0x00, 0x00, 0x02]);
        assert_eq!(mac_from_value(&v).as_deref(), Some("00:1A:2B:00:00:02"));
    }

    #[test]
    fn multicast_macs_are_not_unicast() {
        assert!(!is_unicast_mac("01:00:5E:00:00:01"));
        assert!(is_unicast_mac("00:1A:2B:00:00:02"));
    }

    #[tokio::test]
    async fn sysname_timeout_is_propagated_not_rewritten() {
        let sess = crate::snmp::FixtureSession::failing(TopologyError::Timeout);
        let err = collect_device(&sess, Ipv4Addr::new(192, 168, 1, 2), Some(2))
            .await
            .unwrap_err();
        assert!(matches!(err, TopologyError::Timeout));
        assert!(!err.to_string().to_ascii_lowercase().contains("public"));
    }

    #[tokio::test]
    async fn bad_credentials_stay_auth_failed() {
        let sess = crate::snmp::FixtureSession::failing(TopologyError::AuthFailed);
        let err = collect_device(&sess, Ipv4Addr::new(192, 168, 1, 2), Some(2))
            .await
            .unwrap_err();
        assert!(matches!(err, TopologyError::AuthFailed));
        let msg = err.to_string().to_ascii_lowercase();
        assert!(!msg.contains("community"));
        assert!(!msg.contains("password"));
    }

    #[tokio::test]
    async fn partial_snmp_without_lldp_still_collects_interfaces() {
        let mut t = BTreeMap::new();
        t.insert(
            Oid::from_slice(SYS_NAME).to_dotted(),
            SnmpValue::OctetString(b"core-sw".to_vec()),
        );
        t.insert(
            format!("{}.1", Oid::from_slice(IF_DESCR)),
            SnmpValue::OctetString(b"Gi1/0/1".to_vec()),
        );
        t.insert(
            format!("{}.1", Oid::from_slice(IF_HIGH_SPEED)),
            SnmpValue::Gauge32(1000),
        );
        t.insert(
            format!("{}.1", Oid::from_slice(IF_OPER_STATUS)),
            SnmpValue::Integer(1),
        );
        let view = collect_device(
            &crate::snmp::FixtureSession::new(t),
            Ipv4Addr::new(192, 168, 1, 2),
            Some(2),
        )
        .await
        .unwrap();
        assert_eq!(view.sys_name.as_deref(), Some("core-sw"));
        assert_eq!(view.interfaces.len(), 1);
        assert_eq!(view.interfaces[&1].speed_mbps, Some(1000));
        assert!(view.lldp_neighbors.is_empty());
        assert!(view.mibs_present.iter().any(|m| m == "IF-MIB"));
        assert!(!view.mibs_present.iter().any(|m| m == "LLDP-MIB"));
    }

    #[tokio::test]
    async fn poe_and_vlan_trunk_are_collected() {
        let mut t = BTreeMap::new();
        t.insert(
            Oid::from_slice(SYS_NAME).to_dotted(),
            SnmpValue::OctetString(b"core-sw".to_vec()),
        );
        t.insert(
            format!("{}.12", Oid::from_slice(IF_NAME)),
            SnmpValue::OctetString(b"Port 12".to_vec()),
        );
        t.insert(
            format!("{}.12", Oid::from_slice(DOT1Q_PVID)),
            SnmpValue::Integer(10),
        );
        // VLAN 20 egress includes port 12 (octet 1, bit 3 from MSB → 0x10).
        t.insert(
            format!("{}.20", Oid::from_slice(DOT1Q_VLAN_CURRENT_EGRESS)),
            SnmpValue::OctetString(vec![0x00, 0x10]),
        );
        t.insert(
            format!("{}.1.12", Oid::from_slice(PETH_PSE_DETECTION)),
            SnmpValue::Integer(3),
        );
        t.insert(
            format!("{}.12", Oid::from_slice(CPE_EXT_PWR_ALLOCATED)),
            SnmpValue::Gauge32(8200),
        );
        let view = collect_device(
            &crate::snmp::FixtureSession::new(t),
            Ipv4Addr::new(192, 168, 1, 2),
            Some(2),
        )
        .await
        .unwrap();
        assert_eq!(view.pvid.get(&12).copied(), Some(10));
        assert!(view.tagged.get(&12).is_some_and(|s| s.contains(&20)));
        let poe = view.interfaces[&12].poe.as_ref().expect("poe");
        assert!(poe.enabled);
        assert_eq!(poe.watts, Some(8.2));
        let (vlan, native, tagged) = view.vlan_for_port(12);
        assert_eq!(vlan.as_deref(), Some("trunk"));
        assert_eq!(native, Some(10));
        assert_eq!(tagged, vec![20]);
    }

    #[tokio::test]
    async fn netgear_mojibake_ifalias_falls_back_to_ifname() {
        let mut t = BTreeMap::new();
        t.insert(
            Oid::from_slice(SYS_NAME).to_dotted(),
            SnmpValue::OctetString(b"netgear-sw".to_vec()),
        );
        t.insert(
            format!("{}.7", Oid::from_slice(IF_NAME)),
            SnmpValue::OctetString(b"g7".to_vec()),
        );
        t.insert(
            format!("{}.7", Oid::from_slice(IF_ALIAS)),
            SnmpValue::OctetString(vec![0x80, b'=', 0xC3, 0xBC, b')']),
        );
        t.insert(
            format!("{}.7", Oid::from_slice(IF_DESCR)),
            SnmpValue::OctetString(b"Unit: 1 Slot: 0 Port: 7 Gigabit".to_vec()),
        );
        t.insert(
            format!("{}.18", Oid::from_slice(IF_NAME)),
            SnmpValue::OctetString(b"Gi1/0/18".to_vec()),
        );
        t.insert(
            format!("{}.24", Oid::from_slice(IF_ALIAS)),
            SnmpValue::OctetString(vec![0xFF, 0xFE, 0x00, 0x7D]),
        );
        let view = collect_device(
            &crate::snmp::FixtureSession::new(t),
            Ipv4Addr::new(192, 168, 60, 2),
            Some(2),
        )
        .await
        .unwrap();
        assert_eq!(view.port_name(7), "g7");
        assert_eq!(view.interfaces[&7].display_name(), "g7");
        assert!(view.interfaces[&7]
            .alias
            .as_ref()
            .is_none_or(|s| !s.contains('\u{FFFD}')));
        assert_eq!(view.port_name(18), "Gi1/0/18");
        assert_eq!(view.port_name(24), "24");
        assert!(view
            .notes
            .iter()
            .any(|n| n.contains("ifAlias") && n.contains("7")));
        assert!(!view.port_name(7).contains('\u{FFFD}'));
    }

    #[test]
    fn display_name_prefers_ifname_over_alias() {
        let iface = Iface {
            index: 18,
            name: Some("Gi1/0/18".into()),
            alias: Some("AP-01".into()),
            descr: Some("GigabitEthernet1/0/18".into()),
            mac: None,
            if_type: Some(6),
            admin_status: Some(1),
            oper_status: Some(1),
            speed_mbps: Some(1000),
            poe: None,
        };
        assert_eq!(iface.display_name(), "Gi1/0/18");
    }

    fn test_iface(
        index: u32,
        name: Option<&str>,
        alias: Option<&str>,
        descr: Option<&str>,
    ) -> Iface {
        Iface {
            index,
            name: name.map(str::to_string),
            alias: alias.map(str::to_string),
            descr: descr.map(str::to_string),
            mac: None,
            if_type: Some(6),
            admin_status: Some(1),
            oper_status: Some(1),
            speed_mbps: Some(1000),
            poe: None,
        }
    }

    #[test]
    fn lldp_locport_maps_to_ifindex_then_ifname_wins() {
        let mut view = DeviceView::new(Ipv4Addr::new(192, 168, 1, 2), Some(2));
        view.lldp_local_ports.insert(7, "7".into());
        view.bridge_port_if.insert(7, 18);
        view.interfaces.insert(
            18,
            test_iface(
                18,
                Some("Gi1/0/7"),
                Some("Uplink"),
                Some("GigabitEthernet1/0/7"),
            ),
        );
        view.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 7,
            chassis_id: Some("00:20:AA:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("X0".into()),
            port_desc: None,
            sys_name: Some("fw".into()),
            sys_desc: None,
            management_address: None,
        });
        assert_eq!(view.resolve_if_index(7), 18);
        assert_eq!(view.port_name(7), "Gi1/0/7");
        assert_ne!(view.port_name(7), "7");
    }

    #[test]
    fn ifname_beats_numeric_lldp_local_port_text_on_the_same_ifindex() {
        let mut view = DeviceView::new(Ipv4Addr::new(192, 168, 1, 2), Some(2));
        view.lldp_local_ports.insert(7, "7".into());
        view.interfaces.insert(
            7,
            test_iface(7, Some("Gi1/0/7"), None, Some("GigabitEthernet1/0/7")),
        );
        assert_eq!(view.resolve_if_index(7), 7);
        assert_eq!(view.port_name(7), "Gi1/0/7");
    }
}
