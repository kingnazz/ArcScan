//! Topology evidence providers.
//!
//! Standards first. SNMP (and the LLDP/CDP/FDB/ARP/VLAN/PoE tables it exposes)
//! is implemented. UniFi controller, SonicWall API and manual technician data
//! plug in here later without touching the correlator.

use super::collect::DeviceView;
use super::error::TopologyError;
use super::snmp::SnmpSession;

/// Who produced a piece of topology evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Snmp,
    /// Reserved: UniFi controller API. Not implemented in this PR.
    UnifiController,
    /// Reserved: SonicWall API. Not implemented in this PR.
    SonicwallApi,
    /// Reserved: technician-entered links.
    Manual,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snmp => "snmp",
            Self::UnifiController => "unifi",
            Self::SonicwallApi => "sonicwall",
            Self::Manual => "manual",
        }
    }
}

/// A source of per-device topology evidence. Implementors must isolate
/// failures: one device returning an error must not poison the others.
pub trait TopologyProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn collect<'a>(
        &'a self,
        session: &'a dyn SnmpSession,
        ip: std::net::Ipv4Addr,
        inventory_id: Option<i64>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DeviceView, TopologyError>> + Send + 'a>,
    >;
}

#[derive(Clone, Copy)]
pub struct SnmpProvider;

impl TopologyProvider for SnmpProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Snmp
    }

    fn collect<'a>(
        &'a self,
        session: &'a dyn SnmpSession,
        ip: std::net::Ipv4Addr,
        inventory_id: Option<i64>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DeviceView, TopologyError>> + Send + 'a>,
    > {
        Box::pin(async move { super::collect::collect_device(session, ip, inventory_id).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_sources_are_named_without_being_implemented() {
        assert_eq!(ProviderKind::UnifiController.as_str(), "unifi");
        assert_eq!(ProviderKind::SonicwallApi.as_str(), "sonicwall");
        assert_eq!(ProviderKind::Manual.as_str(), "manual");
        assert_eq!(ProviderKind::Snmp.as_str(), "snmp");
    }
}
