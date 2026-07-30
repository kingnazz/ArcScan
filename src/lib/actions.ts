// Which device actions make sense for a given host.
//
// Every action is always listed, so the panel does not change shape from device
// to device, but only the ones the open services actually support are emphasised
// and enabled. An action with nothing to connect to explains why rather than
// failing after the click.

import { serviceLabel, webPort } from "./format";
import type { HostResult } from "../types";

export type ActionId = "copy" | "web" | "smb" | "rdp" | "ssh" | "wol";

export interface DeviceAction {
  id: ActionId;
  label: string;
  /** True when an open service (or a MAC, for Wake-on-LAN) supports this. */
  available: boolean;
  /** Available actions the operator most likely wants are emphasised. */
  emphasised: boolean;
  /** Why the action is disabled, or what it will do. */
  hint: string;
  /** The port the action will use, when one applies. */
  port?: number;
}

/**
 * Build the action list for a host.
 *
 * Copy is always available: an address is always something worth having on the
 * clipboard. Everything else needs evidence.
 */
export function deviceActions(host: HostResult): DeviceAction[] {
  const ports = host.open_ports;
  const web = webPort(ports);
  const rdp = ports.includes(3389);
  const ssh = ports.includes(22);
  const smbPort = ports.includes(445) ? 445 : ports.includes(139) ? 139 : null;
  const hasMac = Boolean(host.mac?.trim());

  return [
    {
      id: "copy",
      label: "Copy IP",
      available: true,
      emphasised: false,
      hint: `Copy ${host.ip} to the clipboard`,
    },
    {
      id: "web",
      label: "Open web interface",
      available: web != null,
      emphasised: web != null,
      hint:
        web != null
          ? `Open ${serviceLabel(web)} on port ${web}`
          : "No web service found on this device",
      port: web ?? undefined,
    },
    {
      id: "smb",
      label: "Open shared folders",
      available: smbPort != null,
      emphasised: smbPort != null,
      hint:
        smbPort != null
          ? `Open file sharing on port ${smbPort}`
          : "No SMB or NetBIOS service found on this device",
      port: smbPort ?? undefined,
    },
    {
      id: "rdp",
      label: "Open Remote Desktop",
      available: rdp,
      emphasised: rdp,
      hint: rdp ? "Connect with RDP on port 3389" : "Port 3389 is not open on this device",
      port: rdp ? 3389 : undefined,
    },
    {
      id: "ssh",
      label: "Open SSH",
      available: ssh,
      emphasised: ssh,
      hint: ssh ? "Open a terminal session on port 22" : "Port 22 is not open on this device",
      port: ssh ? 22 : undefined,
    },
    {
      id: "wol",
      label: "Wake-on-LAN",
      available: hasMac,
      // Waking a device that is already answering is rarely what anyone wants,
      // so it is available but never the emphasised choice.
      emphasised: false,
      hint: hasMac
        ? "Send a Wake-on-LAN magic packet to this device"
        : "Wake-on-LAN needs a MAC address, which is only available on your local segment",
    },
  ];
}

/** The single action the drawer highlights, or null when nothing stands out. */
export function primaryAction(host: HostResult): DeviceAction | null {
  const actions = deviceActions(host);
  // Order matters: remote control beats file sharing beats a web interface,
  // because that is the order of what someone opens a device to do.
  for (const id of ["rdp", "ssh", "smb", "web"] as ActionId[]) {
    const action = actions.find((a) => a.id === id);
    if (action?.available) return action;
  }
  return null;
}
