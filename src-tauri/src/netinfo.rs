//! Local network interface detection, used to auto-fill the scan target with
//! the machine's own subnet (like Advanced IP Scanner / Angry IP Scanner do),
//! and default-gateway detection, used to tell two otherwise identical private
//! subnets apart when resolving a scan's network scope.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::Duration;

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

/// The machine's default IPv4 gateway, read from the OS routing table.
///
/// Best-effort: returns `None` when there is no default route or the routing
/// tool is unavailable. Used only as a scope-identity hint, so failure here
/// never affects a scan.
pub async fn default_gateway_ip() -> Option<Ipv4Addr> {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("route", &["print", "-4", "0.0.0.0"])
    } else if cfg!(target_os = "macos") {
        ("route", &["-n", "get", "default"])
    } else {
        ("ip", &["route", "show", "default"])
    };

    let mut cmd = crate::scanner::quiet_command(program);
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(3), cmd.output())
        .await
        .ok()?
        .ok()?;
    parse_gateway(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the default gateway from the routing-table output of any platform.
///
/// * Linux `ip route show default`: `default via 192.168.1.1 dev eth0`
/// * macOS `route -n get default`: `    gateway: 192.168.1.1`
/// * Windows `route print -4 0.0.0.0`: a row `0.0.0.0  0.0.0.0  192.168.1.1 ...`
fn parse_gateway(text: &str) -> Option<Ipv4Addr> {
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.as_slice() {
            // Linux: default via <gw> ...
            ["default", "via", gw, ..] => {
                if let Ok(ip) = gw.parse() {
                    return Some(ip);
                }
            }
            // macOS: gateway: <gw>
            ["gateway:", gw] => {
                if let Ok(ip) = gw.parse() {
                    return Some(ip);
                }
            }
            // Windows: 0.0.0.0 0.0.0.0 <gw> <iface> <metric>
            ["0.0.0.0", "0.0.0.0", gw, ..] => {
                if let Ok(ip) = gw.parse::<Ipv4Addr>() {
                    if !ip.is_unspecified() {
                        return Some(ip);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_default_route() {
        let out = "default via 192.168.1.1 dev wlan0 proto dhcp metric 600\n";
        assert_eq!(parse_gateway(out), Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn parses_macos_route_get() {
        let out = "   route to: default\ndestination: default\n       mask: default\n    gateway: 10.0.1.1\n  interface: en0\n";
        assert_eq!(parse_gateway(out), Some(Ipv4Addr::new(10, 0, 1, 1)));
    }

    #[test]
    fn parses_windows_route_print() {
        let out = "IPv4 Route Table\n===========\nActive Routes:\nNetwork Destination        Netmask          Gateway       Interface  Metric\n          0.0.0.0          0.0.0.0      192.168.0.254    192.168.0.20     25\n";
        assert_eq!(parse_gateway(out), Some(Ipv4Addr::new(192, 168, 0, 254)));
    }

    #[test]
    fn missing_default_route_yields_none() {
        assert_eq!(parse_gateway(""), None);
        assert_eq!(parse_gateway("10.0.0.0/24 dev eth0 proto kernel\n"), None);
        // An on-link Windows pseudo-gateway is not an address.
        assert_eq!(
            parse_gateway("0.0.0.0 0.0.0.0 On-link 192.168.0.20 25"),
            None
        );
    }
}
