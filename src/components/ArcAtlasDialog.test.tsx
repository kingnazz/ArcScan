import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ArcAtlasDialog, normalizeArcAtlasServerInput } from "./ArcAtlasDialog";
import { DISCONNECTED_CONNECTION, PORTABLE_SESSION_COPY, type ArcAtlasConnection, type ArcAtlasSendResult } from "../lib/arcatlas";

afterEach(() => {
  cleanup();
});

const connected: ArcAtlasConnection = {
  ...DISCONNECTED_CONNECTION,
  configured: true,
  serverUrl: "https://atlas.example.com",
  connectionName: "Onsite",
  clientName: "Cedar Ridge",
  siteName: "Seattle HQ",
  tokenPrefix: "atlas_arcscan_abcd",
};

const result: ArcAtlasSendResult = {
  runId: "run-1",
  recordCount: 42,
  presentCount: 40,
  missingCount: 1,
  unknownCount: 1,
  clientName: "Cedar Ridge",
  siteName: "Seattle HQ",
  discoveryUrl: "https://atlas.example.com/discovery?run=run-1",
  duplicate: false,
  status: 201,
};

const noop = {
  onClose: () => undefined,
  onConfigure: async () => undefined,
  onDisconnect: async () => undefined,
  onReconnect: () => undefined,
  onSend: async () => undefined,
  onRetry: async () => undefined,
  onOpenInArcAtlas: () => undefined,
};

describe("ArcAtlas dialog", () => {
  it("disconnected send path is the connection setup, not an automatic send", () => {
    render(<ArcAtlasDialog {...noop} open mode="connect" connection={DISCONNECTED_CONNECTION} />);
    expect(screen.getByText("Connect ArcAtlas")).toBeTruthy();
    expect(screen.queryByText("Send to ArcAtlas")).toBeNull();
  });

  it("clears the token field after a successful connection", async () => {
    const onConfigure = vi.fn(async () => undefined);
    render(<ArcAtlasDialog {...noop} open mode="connect" connection={DISCONNECTED_CONNECTION} onConfigure={onConfigure} />);
    fireEvent.change(screen.getByLabelText("ArcAtlas server URL"), { target: { value: "https://atlas.example.com" } });
    fireEvent.change(screen.getByLabelText("Connection token"), { target: { value: "atlas_arcscan_supersecret" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    await waitFor(() => {
      expect(onConfigure).toHaveBeenCalledWith("https://atlas.example.com", "atlas_arcscan_supersecret");
      expect((screen.getByLabelText("Connection token") as HTMLInputElement).value).toBe("");
    });
  });

  it("normalizes the copied ArcAtlas machine endpoint to its server URL", async () => {
    expect(normalizeArcAtlasServerInput("https://atlas.example.com/api/discovery/arcscan")).toBe(
      "https://atlas.example.com",
    );
    expect(normalizeArcAtlasServerInput("https://atlas.example.com/api/discovery/arcscan/?copied=1#test")).toBe(
      "https://atlas.example.com",
    );

    const onConfigure = vi.fn(async () => undefined);
    render(<ArcAtlasDialog {...noop} open mode="connect" connection={DISCONNECTED_CONNECTION} onConfigure={onConfigure} />);
    fireEvent.change(screen.getByLabelText("ArcAtlas server URL"), {
      target: { value: "https://atlas.example.com/api/discovery/arcscan" },
    });
    fireEvent.change(screen.getByLabelText("Connection token"), {
      target: { value: "atlas_arcscan_supersecret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    await waitFor(() => {
      expect(onConfigure).toHaveBeenCalledWith("https://atlas.example.com", "atlas_arcscan_supersecret");
      expect((screen.getByLabelText("ArcAtlas server URL") as HTMLInputElement).value).toBe(
        "https://atlas.example.com",
      );
    });
  });

  it("renders the connected destination and only a token prefix", () => {
    render(<ArcAtlasDialog {...noop} open mode="status" connection={connected} />);
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.getByText("Cedar Ridge / Seattle HQ")).toBeTruthy();
    expect(screen.getByText(/atlas_arcscan_abcd/)).toBeTruthy();
    expect(screen.queryByText("atlas_arcscan_supersecret")).toBeNull();
  });

  it("shows portable session-only copy", () => {
    render(<ArcAtlasDialog {...noop} open mode="status" connection={{ ...connected, portableSessionOnly: true }} />);
    expect(screen.getByText(PORTABLE_SESSION_COPY)).toBeTruthy();
  });

  it("confirmation shows network, destination and device count", () => {
    render(
      <ArcAtlasDialog
        {...noop}
        open
        mode="confirm"
        connection={connected}
        confirmation={{
          destination: "Cedar Ridge / Seattle HQ",
          networkName: "192.168.10.0/24",
          deviceCount: 42,
          explanation: "Sends observed inventory to ArcAtlas Discovery. It does not change documented devices.",
        }}
      />,
    );
    expect(screen.getByText("192.168.10.0/24")).toBeTruthy();
    expect(screen.getByText("42")).toBeTruthy();
    expect(screen.queryByText(/online|offline|down/i)).toBeNull();
  });

  it("renders success counts and opens the returned discovery URL", () => {
    const onOpenInArcAtlas = vi.fn();
    render(
      <ArcAtlasDialog {...noop} open mode="success" connection={connected} result={result} onOpenInArcAtlas={onOpenInArcAtlas} />,
    );
    expect(screen.getByText("Sent to ArcAtlas")).toBeTruthy();
    expect(screen.getByText("Observed: 42")).toBeTruthy();
    expect(screen.getByText("Not observed: 1")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open in ArcAtlas" }));
    expect(onOpenInArcAtlas).toHaveBeenCalledWith(result.discoveryUrl);
  });

  it("shows reconfigure after 401 and retry after timeout", () => {
    const onReconnect = vi.fn();
    const { rerender } = render(
      <ArcAtlasDialog
        {...noop}
        open
        mode="error"
        connection={connected}
        error={{ code: "unauthorized", message: "The ArcAtlas connection token is invalid or revoked.", retryable: false }}
        onReconnect={onReconnect}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Reconfigure" }));
    expect(onReconnect).toHaveBeenCalled();
    const onRetry = vi.fn();
    rerender(
      <ArcAtlasDialog
        {...noop}
        open
        mode="error"
        connection={connected}
        error={{ code: "timeout", message: "The ArcAtlas request timed out.", retryable: true }}
        onRetry={onRetry}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalled();
  });
});
