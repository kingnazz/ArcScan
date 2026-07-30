import { describe, expect, it } from "vitest";
import { deviceActions, primaryAction } from "./actions";
import type { HostResult } from "../types";

function host(open_ports: number[], mac: string | null = "AA:BB:CC:00:00:01"): HostResult {
  return {
    ip: "10.0.0.5",
    hostname: null,
    mac,
    vendor: null,
    open_ports,
    response_ms: 2,
    icmp_ms: 1.5,
    tcp_ms: 2.1,
    ttl: 64,
    os_guess: null,
    last_seen: "2026-07-01T10:00:00Z",
  };
}

const byId = (h: HostResult) => new Map(deviceActions(h).map((a) => [a.id, a]));

describe("device action availability", () => {
  it("always offers Copy IP", () => {
    expect(byId(host([])).get("copy")?.available).toBe(true);
  });

  it("enables RDP only when 3389 is open", () => {
    expect(byId(host([3389])).get("rdp")?.available).toBe(true);
    expect(byId(host([3389])).get("rdp")?.emphasised).toBe(true);
    expect(byId(host([22, 443])).get("rdp")?.available).toBe(false);
  });

  it("enables SSH only when 22 is open", () => {
    expect(byId(host([22])).get("ssh")?.available).toBe(true);
    expect(byId(host([3389])).get("ssh")?.available).toBe(false);
  });

  it("enables shared folders for either SMB or NetBIOS", () => {
    expect(byId(host([445])).get("smb")?.port).toBe(445);
    expect(byId(host([139])).get("smb")?.port).toBe(139);
    // 445 wins when both are open, because it is the modern one.
    expect(byId(host([139, 445])).get("smb")?.port).toBe(445);
    expect(byId(host([22])).get("smb")?.available).toBe(false);
  });

  it("prefers HTTPS for the web interface", () => {
    expect(byId(host([80, 443])).get("web")?.port).toBe(443);
    expect(byId(host([80])).get("web")?.port).toBe(80);
    expect(byId(host([22])).get("web")?.available).toBe(false);
  });

  it("disables Wake-on-LAN without a MAC address and explains why", () => {
    const withoutMac = byId(host([80], null)).get("wol");
    expect(withoutMac?.available).toBe(false);
    expect(withoutMac?.hint).toMatch(/MAC address/);
    expect(byId(host([80])).get("wol")?.available).toBe(true);
  });

  it("never emphasises Wake-on-LAN, since the device is already answering", () => {
    expect(byId(host([80])).get("wol")?.emphasised).toBe(false);
  });

  it("explains why a disabled action is disabled", () => {
    const actions = deviceActions(host([80]));
    for (const action of actions.filter((a) => !a.available)) {
      expect(action.hint.length, action.id).toBeGreaterThan(10);
    }
  });

  it("lists the same actions in the same order for every device", () => {
    const order = deviceActions(host([])).map((a) => a.id);
    expect(deviceActions(host([22, 80, 445, 3389])).map((a) => a.id)).toEqual(order);
    expect(order).toEqual(["copy", "web", "smb", "rdp", "ssh", "wol"]);
  });
});

describe("primary action selection", () => {
  it("ranks remote control above file sharing above a web interface", () => {
    expect(primaryAction(host([80, 443, 445, 22, 3389]))?.id).toBe("rdp");
    expect(primaryAction(host([80, 443, 445, 22]))?.id).toBe("ssh");
    expect(primaryAction(host([80, 443, 445]))?.id).toBe("smb");
    expect(primaryAction(host([80, 443]))?.id).toBe("web");
  });

  it("emphasises nothing when no service supports an action", () => {
    expect(primaryAction(host([]))).toBeNull();
    expect(primaryAction(host([53]))).toBeNull();
  });
});
