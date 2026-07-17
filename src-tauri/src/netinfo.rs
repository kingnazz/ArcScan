//! Local network interface detection, used to auto-fill the scan target with
//! the machine's own subnet (like Advanced IP Scanner / Angry IP Scanner do).

use std::collections::HashSet;
use std::net::Ipv4Addr;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LocalNetwork {
    /// Interface name (e.g. `en0`, `Ethernet`).
    pub interface: String,
    /// This host's own address on the interface.
    pub ip: String,
    /// CIDR prefix length derived from the netmask.
    pub prefix: u8,
    /// The network in CIDR form, ready to drop into the target field.
    pub cidr: String,
    pub is_private: bool,
}

/// Enumerate usable IPv4 interfaces and return them as scan-ready CIDRs.
/// Loopback and link-local (169.254/16) addresses are skipped. Private,
/// most-specific subnets are returned first so the UI can pick the best default.
pub fn detect() -> Vec<LocalNetwork> {
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut nets: Vec<LocalNetwork> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for iface in ifaces {
        let if_addrs::IfAddr::V4(v4) = iface.addr else {
            continue;
        };
        let ip = v4.ip;
        if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
            continue;
        }
        let mask = u32::from(v4.netmask);
        let prefix = mask.count_ones() as u8;
        // Ignore nonsensical masks (e.g. /0 or a host-only /32 on some VPNs).
        if prefix == 0 || prefix > 30 {
            continue;
        }
        let network = Ipv4Addr::from(u32::from(ip) & mask);
        let cidr = format!("{network}/{prefix}");
        if !seen.insert(cidr.clone()) {
            continue;
        }
        nets.push(LocalNetwork {
            interface: iface.name.clone(),
            ip: ip.to_string(),
            prefix,
            cidr,
            is_private: ip.is_private(),
        });
    }

    // Order: private first, then larger networks (smaller prefix), then by
    // interface name for stability — so the UI's first pick is the best default.
    nets.sort_by(|a, b| {
        b.is_private
            .cmp(&a.is_private)
            .then(a.prefix.cmp(&b.prefix).reverse())
            .then(a.interface.cmp(&b.interface))
    });
    nets
}
