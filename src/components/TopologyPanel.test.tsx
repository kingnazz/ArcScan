import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TopologyPanel } from "./TopologyPanel";
import { EMPTY_CREDENTIAL_STATUS, type CredentialInput, type TopologyResult } from "../lib/topology";

afterEach(() => {
  cleanup();
});

const names = {
  byId: new Map<number, string>([
    [1, "Home Router"],
    [2, "Core Switch"],
    [4, "Home NAS"],
  ]),
  byIp: new Map<string, string>(),
};

const result: TopologyResult = {
  snapshot: {
    capturedAt: "2026-09-16T12:00:00Z",
    connections: [
      {
        fromDeviceId: 2,
        toDeviceId: 1,
        fromPort: "Port 48",
        toPort: "X0",
        kind: "ethernet",
        protocol: "lldp",
        confidence: "confirmed",
        speedMbps: 1000,
        vlan: "trunk",
        nativeVlan: 10,
        taggedVlans: [10, 20, 30],
        evidence: ["LLDP neighbour on Core Switch port 48 reports SonicWall X0"],
      },
      {
        fromDeviceId: 2,
        toDeviceId: 4,
        fromPort: "Port 20",
        kind: "ethernet",
        protocol: "fdb",
        confidence: "strong",
        speedMbps: 1000,
        vlan: "10",
        nativeVlan: 10,
        taggedVlans: [],
        poe: { enabled: true, watts: 8.2 },
        evidence: ["Exactly one unicast MAC learned on access port Port 20"],
      },
    ],
    unknownNodes: [],
  },
  summary: {
    devicesQueried: 5,
    devicesResponded: 1,
    devicesFailed: 4,
    confirmed: 1,
    strong: 1,
    inferred: 0,
    unknownNodes: 0,
    durationMs: 900,
    cancelled: false,
    timedOut: false,
    failures: [{ ip: "192.168.1.50", reason: "The device did not answer SNMP in time." }],
  },
};

const noop = {
  onSaveCredentials: async () => undefined,
  onClearCredentials: async () => undefined,
  onDiscover: async () => undefined,
  onCancel: () => undefined,
  onBack: () => undefined,
};

describe("Topology panel", () => {
  it("does not offer discovery until credentials are configured", () => {
    render(
      <TopologyPanel
        {...noop}
        credentialStatus={EMPTY_CREDENTIAL_STATUS}
        result={null}
        names={names}
        targetCount={4}
        busy={false}
        error={null}
      />,
    );
    expect((screen.getByRole("button", { name: "Discover topology" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(screen.getByText(/never tries/i).textContent?.toLowerCase()).toContain("public");
  });

  it("clears secret fields after saving credentials for the session", async () => {
    const onSaveCredentials = vi.fn(async (_input: CredentialInput) => undefined);
    render(
      <TopologyPanel
        {...noop}
        onSaveCredentials={onSaveCredentials}
        credentialStatus={EMPTY_CREDENTIAL_STATUS}
        result={null}
        names={names}
        targetCount={4}
        busy={false}
        error={null}
      />,
    );
    fireEvent.change(screen.getByLabelText("Community"), { target: { value: "site-read-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Keep for this session" }));
    await waitFor(() => {
      expect(onSaveCredentials).toHaveBeenCalled();
      expect((screen.getByLabelText("Community") as HTMLInputElement).value).toBe("");
    });
    const payload = onSaveCredentials.mock.calls[0]?.[0];
    expect(payload?.community).toBe("site-read-secret");
    expect(payload?.version).toBe("v2c");
  });

  it("renders a concise result without echoing credentials", () => {
    render(
      <TopologyPanel
        {...noop}
        credentialStatus={{
          configured: true,
          version: "v2c",
          username: null,
          authProtocol: null,
          privProtocol: null,
          sessionOnly: true,
        }}
        result={result}
        names={names}
        targetCount={5}
        busy={false}
        error={null}
      />,
    );
    expect(screen.getByText(/1 confirmed · 1 strong · 0 inferred/)).toBeTruthy();
    expect(screen.getByText((_, node) => node?.tagName === "P" && (node.textContent ?? "").includes("Home Router"))).toBeTruthy();
    expect(screen.getByText((_, node) => node?.tagName === "P" && (node.textContent ?? "").includes("Home NAS"))).toBeTruthy();
    expect(screen.getByText("Confirmed")).toBeTruthy();
    expect(screen.getByText(/PoE/)).toBeTruthy();
    expect(screen.queryByText("site-read")).toBeNull();
  });

  it("blocks empty community without spraying a default", () => {
    render(
      <TopologyPanel
        {...noop}
        credentialStatus={EMPTY_CREDENTIAL_STATUS}
        result={null}
        names={names}
        targetCount={4}
        busy={false}
        error={null}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Keep for this session" }));
    expect(screen.getByRole("alert").textContent).toMatch(/community string/i);
    expect(screen.getByRole("alert").textContent?.toLowerCase()).not.toContain("try public");
  });
});
