// ArcAtlas direct handoff helpers.
//
// The Inventory JSON exporter is the only device mapper. This module wraps that
// output in the transport envelope, keeps one handoff id for a retry, and maps
// Rust errors into short UI copy. It never stores or returns the connection token.

import { buildInventoryExport } from "./export";
import type { InventoryRow } from "../types";
import { APP_VERSION } from "../version";

export const PORTABLE_SESSION_COPY = "Portable: ArcAtlas connection is kept for this session only.";
export const SEND_EXPLANATION =
  "Sends observed inventory to ArcAtlas Discovery. It does not change documented devices.";
export const DISCONNECT_WARNING =
  "Disconnecting ArcScan does not revoke the ArcAtlas token. Revoke the token from ArcAtlas.";

export interface ArcAtlasConnection {
  configured: boolean;
  serverUrl: string | null;
  connectionName: string | null;
  clientName: string | null;
  siteName: string | null;
  tokenPrefix: string | null;
  lastValidatedAt: string | null;
  portableSessionOnly: boolean;
  needsReconfigure: boolean;
}

export interface ArcAtlasSendResult {
  runId: string;
  recordCount: number;
  presentCount: number;
  missingCount: number;
  unknownCount: number;
  clientName: string;
  siteName: string;
  discoveryUrl: string;
  duplicate: boolean;
  status: number;
}

export type ArcAtlasErrorCode =
  | "invalid_url"
  | "insecure_http"
  | "unsupported_scheme"
  | "redirect"
  | "timeout"
  | "unauthorized"
  | "payload_too_large"
  | "validation"
  | "malformed"
  | "server"
  | "network"
  | "not_configured"
  | "internal";

export interface ArcAtlasError {
  code: ArcAtlasErrorCode;
  message: string;
  retryable: boolean;
}

export interface ArcAtlasHandoffEnvelope {
  schemaVersion: 1;
  handoffId: string;
  sourceVersion: string;
  generatedAt: string;
  networkName: string;
  inventory: unknown[];
}

export interface SendConfirmation {
  destination: string;
  networkName: string;
  deviceCount: number;
  explanation: string;
}

export const DISCONNECTED_CONNECTION: ArcAtlasConnection = {
  configured: false,
  serverUrl: null,
  connectionName: null,
  clientName: null,
  siteName: null,
  tokenPrefix: null,
  lastValidatedAt: null,
  portableSessionOnly: false,
  needsReconfigure: false,
};

export function destinationLabel(connection: Pick<ArcAtlasConnection, "clientName" | "siteName">): string {
  const client = connection.clientName?.trim() ?? "";
  const site = connection.siteName?.trim() ?? "";
  if (client && site) return `${client} / ${site}`;
  return client || site || "ArcAtlas";
}

export function displayTokenPrefix(prefix: string | null | undefined): string | null {
  if (!prefix) return null;
  const trimmed = prefix.trim();
  if (!trimmed) return null;
  return trimmed.endsWith("...") ? trimmed : `${trimmed}...`;
}

export function selectedNetworkName(
  rows: InventoryRow[],
  networkId: number | null,
  networks: Array<{ id: number; name: string }>,
): string | null {
  if (networkId != null) {
    return networks.find((network) => network.id === networkId)?.name ?? rows[0]?.network_name ?? null;
  }
  const names = uniqueNetworkNames(rows);
  return names.length === 1 ? names[0] : null;
}

export function uniqueNetworkNames(rows: InventoryRow[]): string[] {
  return [...new Set(rows.map((row) => row.network_name).filter((name): name is string => Boolean(name)))];
}

export function canSendSingleNetwork(args: {
  networkId: number | null;
  networkCount: number;
  rows: InventoryRow[];
}): boolean {
  if (args.rows.length === 0) return false;
  if (args.networkCount > 1 && args.networkId == null) return false;
  return uniqueNetworkNames(args.rows).length <= 1;
}

export function nextModeOnSend(
  connection: ArcAtlasConnection,
  canSend: boolean,
): "connect" | "confirm" | "choose-network" {
  if (!connection.configured || connection.needsReconfigure) return "connect";
  if (!canSend) return "choose-network";
  return "confirm";
}

export function sendConfirmation(args: {
  connection: ArcAtlasConnection;
  networkName: string;
  deviceCount: number;
}): SendConfirmation {
  return {
    destination: destinationLabel(args.connection),
    networkName: args.networkName,
    deviceCount: args.deviceCount,
    explanation: SEND_EXPLANATION,
  };
}

export function buildHandoffEnvelope(args: {
  rows: InventoryRow[];
  notes: Map<number, string>;
  networkName: string;
  handoffId: string;
  generatedAt?: string;
  sourceVersion?: string;
}): ArcAtlasHandoffEnvelope {
  const exported = buildInventoryExport(args.rows, "json", args.notes);
  const inventory = JSON.parse(exported) as unknown[];
  return {
    schemaVersion: 1,
    handoffId: args.handoffId,
    sourceVersion: args.sourceVersion ?? APP_VERSION,
    generatedAt: args.generatedAt ?? new Date().toISOString(),
    networkName: args.networkName,
    inventory,
  };
}

export class HandoffAttempt {
  private activeId: string | null = null;

  begin(createId: () => string = createHandoffId): string {
    if (!this.activeId) this.activeId = createId();
    return this.activeId;
  }

  peek(): string | null {
    return this.activeId;
  }

  succeed(): void {
    this.activeId = null;
  }

  failRetryable(): void {}

  reset(): void {
    this.activeId = null;
  }
}

export function createHandoffId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return "00000000-0000-4000-8000-000000000000";
}

export function parseArcAtlasError(error: unknown): ArcAtlasError {
  const raw = stringifyError(error);
  const parsed = tryParseJson(raw);
  const code = (parsed?.code as ArcAtlasErrorCode | undefined) ?? inferCode(raw);
  const message = sanitizeUserMessage(parsed?.message ?? fallbackMessage(code));
  return {
    code,
    message,
    retryable: code === "timeout" || code === "network" || code === "server" || code === "redirect",
  };
}

export function successCounts(result: ArcAtlasSendResult): {
  observed: number;
  present: number;
  notObserved: number;
  unknown: number;
} {
  return {
    observed: result.recordCount,
    present: result.presentCount,
    notObserved: result.missingCount,
    unknown: result.unknownCount,
  };
}

export function presenceCopyIsConservative(text: string): boolean {
  return !/\b(online|offline|down|unreachable)\b/i.test(text);
}

function stringifyError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "";
}

function tryParseJson(raw: string): { code?: string; message?: string } | null {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    return JSON.parse(trimmed) as { code?: string; message?: string };
  } catch {
    return null;
  }
}

function inferCode(raw: string): ArcAtlasErrorCode {
  const text = raw.toLowerCase();
  if (text.includes("401") || text.includes("unauthorized") || text.includes("revoked")) return "unauthorized";
  if (text.includes("413") || text.includes("too large")) return "payload_too_large";
  if (text.includes("422")) return "validation";
  if (text.includes("timed out") || text.includes("timeout")) return "timeout";
  if (text.includes("redirect")) return "redirect";
  if (text.includes("not configured")) return "not_configured";
  return "internal";
}

function fallbackMessage(code: ArcAtlasErrorCode): string {
  switch (code) {
    case "invalid_url":
      return "Enter a valid ArcAtlas server URL.";
    case "insecure_http":
      return "ArcAtlas requires HTTPS, except on localhost.";
    case "unsupported_scheme":
      return "Only HTTP and HTTPS server URLs are allowed.";
    case "redirect":
      return "The ArcAtlas server redirected the request. Refusing to follow it.";
    case "timeout":
      return "The ArcAtlas request timed out.";
    case "unauthorized":
      return "The ArcAtlas connection token is invalid or revoked.";
    case "payload_too_large":
      return "The inventory is too large for ArcAtlas to accept.";
    case "validation":
      return "ArcAtlas rejected the inventory or network selection.";
    case "malformed":
      return "The request was not accepted by ArcAtlas.";
    case "server":
      return "ArcAtlas could not complete the request.";
    case "network":
      return "Could not reach the ArcAtlas server.";
    case "not_configured":
      return "Connect ArcAtlas before sending inventory.";
    default:
      return "The ArcAtlas connection failed.";
  }
}

export function sanitizeUserMessage(message: string): string {
  return message
    .replace(/Bearer\s+\S+/gi, "Bearer [redacted]")
    .replace(/atlas_arcscan_[A-Za-z0-9._-]+/g, "atlas_arcscan_[redacted]");
}
