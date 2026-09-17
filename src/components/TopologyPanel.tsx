// Topology discovery: SNMP credentials, a run button, a concise result.
//
// This is a Deep Scan / credentialed step. Quick LAN does not run it. The
// panel is deliberately small — ArcAtlas owns the map.

import { useState } from "react";
import { Network, Square } from "lucide-react";
import { Badge, Button, Field, FieldRow, Select } from "../ui/primitives";
import { Tooltip } from "../ui/Popover";
import { TopologyPreview } from "./TopologyPreview";
import {
  CONFIDENCE_HINT,
  SNMP_AUTH_PROTOCOLS,
  SNMP_PRIV_PROTOCOLS,
  TOPOLOGY_HINT,
  confidenceLabel,
  credentialInputError,
  emptyCredentialInput,
  endpointLabel,
  protocolLabel,
  speedLabel,
  summaryLine,
  vlanLabel,
  type CredentialInput,
  type CredentialStatus,
  type DeviceNameLookup,
  type DeviceTypeLookup,
  type TopologyResult,
  type UnresolvedNode,
} from "../lib/topology";

export interface TopologyPanelProps {
  credentialStatus: CredentialStatus;
  result: TopologyResult | null;
  names: DeviceNameLookup;
  types?: DeviceTypeLookup;
  targetCount: number;
  busy: boolean;
  error: string | null;
  onSaveCredentials: (input: CredentialInput) => Promise<void> | void;
  onClearCredentials: () => Promise<void> | void;
  onDiscover: () => Promise<void> | void;
  onCancel: () => void;
  onBack: () => void;
}

export function TopologyPanel({
  credentialStatus,
  result,
  names,
  types = { byId: new Map() },
  targetCount,
  busy,
  error,
  onSaveCredentials,
  onClearCredentials,
  onDiscover,
  onCancel,
  onBack,
}: TopologyPanelProps) {
  const [form, setForm] = useState<CredentialInput>(() => emptyCredentialInput("v2c"));
  const [formError, setFormError] = useState<string | null>(null);

  const save = async () => {
    const issue = credentialInputError(form);
    if (issue) {
      setFormError(issue);
      return;
    }
    setFormError(null);
    await onSaveCredentials(form);
    setForm((current) => ({
      ...current,
      community: "",
      authPassword: "",
      privPassword: "",
    }));
  };

  const unknownNodes: UnresolvedNode[] = result?.snapshot.unknownNodes ?? [];

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="mx-auto max-w-5xl space-y-4 px-4 py-4">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold text-text">
              <Tooltip content={TOPOLOGY_HINT}>
                <span tabIndex={0}>Topology discovery</span>
              </Tooltip>
            </h2>
            <p className="mt-1 text-[13px] leading-relaxed text-text-secondary">
              Optional credentialed SNMP discovery for the devices saved by this scan. It only runs
              when you start it; Quick discovery never starts it automatically. Credentials stay in
              this session and are never exported.
            </p>
          </div>
          <Button size="sm" variant="ghost" onClick={onBack}>
            Back to devices
          </Button>
        </div>

        <section className="surface-panel p-4">
          <h3 className="text-[13px] font-semibold text-text">SNMP credentials</h3>
          <p className="mt-1 text-xs leading-relaxed text-text-muted">
            Session only — forgotten when ArcScan closes. ArcScan never tries{" "}
            <span className="mono">public</span>, <span className="mono">private</span> or any other
            default community.
          </p>

          {credentialStatus.configured ? (
            <p className="mt-2 text-[13px] text-text-secondary">
              {credentialStatus.version === "v3"
                ? `SNMPv3 is configured for this session${
                    credentialStatus.authProtocol ? ` (${credentialStatus.authProtocol}` : ""
                  }${credentialStatus.privProtocol ? `/${credentialStatus.privProtocol}` : ""}${
                    credentialStatus.authProtocol ? ")" : ""
                  }.`
                : "SNMPv2c is configured for this session."}{" "}
              The community, username and passwords are not shown again.
            </p>
          ) : (
            <p className="mt-2 text-[13px] text-text-secondary">No credentials in this session yet.</p>
          )}

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <FieldRow label="Version" htmlFor="topo-snmp-version">
              <Select
                id="topo-snmp-version"
                value={form.version}
                onChange={(event) =>
                  setForm(emptyCredentialInput(event.target.value === "v3" ? "v3" : "v2c"))
                }
              >
                <option value="v2c">SNMP v2c</option>
                <option value="v3">SNMP v3</option>
              </Select>
            </FieldRow>

            {form.version === "v2c" ? (
              <FieldRow
                label="Community"
                htmlFor="topo-snmp-community"
                hint="The read-only community the switches already use."
              >
                <Field
                  id="topo-snmp-community"
                  type="password"
                  autoComplete="off"
                  value={form.community ?? ""}
                  onChange={(event) => setForm({ ...form, community: event.target.value })}
                />
              </FieldRow>
            ) : (
              <>
                <FieldRow label="Username" htmlFor="topo-snmp-user">
                  <Field
                    id="topo-snmp-user"
                    autoComplete="off"
                    value={form.username ?? ""}
                    onChange={(event) => setForm({ ...form, username: event.target.value })}
                  />
                </FieldRow>
                <FieldRow label="Authentication" htmlFor="topo-snmp-auth">
                  <Select
                    id="topo-snmp-auth"
                    value={form.authProtocol ?? "sha256"}
                    onChange={(event) => setForm({ ...form, authProtocol: event.target.value })}
                  >
                    {SNMP_AUTH_PROTOCOLS.map((proto) => (
                      <option key={proto} value={proto}>
                        {proto.toUpperCase()}
                      </option>
                    ))}
                  </Select>
                </FieldRow>
                <FieldRow label="Authentication password" htmlFor="topo-snmp-auth-pass">
                  <Field
                    id="topo-snmp-auth-pass"
                    type="password"
                    autoComplete="off"
                    value={form.authPassword ?? ""}
                    onChange={(event) => setForm({ ...form, authPassword: event.target.value })}
                  />
                </FieldRow>
                <FieldRow label="Privacy" htmlFor="topo-snmp-priv">
                  <Select
                    id="topo-snmp-priv"
                    value={form.privProtocol ?? ""}
                    onChange={(event) => {
                      const value = event.target.value;
                      setForm({
                        ...form,
                        privProtocol: value,
                        privPassword: value ? form.privPassword : "",
                      });
                    }}
                  >
                    <option value="">None (authNoPriv)</option>
                    {SNMP_PRIV_PROTOCOLS.map((proto) => (
                      <option key={proto} value={proto}>
                        {proto.toUpperCase()}
                      </option>
                    ))}
                  </Select>
                </FieldRow>
                {form.privProtocol ? (
                  <FieldRow label="Privacy password" htmlFor="topo-snmp-priv-pass">
                    <Field
                      id="topo-snmp-priv-pass"
                      type="password"
                      autoComplete="off"
                      value={form.privPassword ?? ""}
                      onChange={(event) => setForm({ ...form, privPassword: event.target.value })}
                    />
                  </FieldRow>
                ) : null}
                <FieldRow
                  label="Context"
                  htmlFor="topo-snmp-context"
                  hint="Optional SNMP context name."
                >
                  <Field
                    id="topo-snmp-context"
                    autoComplete="off"
                    value={form.context ?? ""}
                    onChange={(event) => setForm({ ...form, context: event.target.value })}
                  />
                </FieldRow>
              </>
            )}
          </div>

          {formError ? (
            <p role="alert" className="mt-2 text-xs text-danger">
              {formError}
            </p>
          ) : null}

          <div className="mt-3 flex flex-wrap gap-2">
            <Button size="sm" variant="secondary" onClick={() => void save()} disabled={busy}>
              Keep for this session
            </Button>
            {credentialStatus.configured ? (
              <Button size="sm" variant="ghost" onClick={() => void onClearCredentials()} disabled={busy}>
                Forget credentials
              </Button>
            ) : null}
          </div>
        </section>

        <section className="surface-panel p-4">
          <div className="flex flex-wrap items-center gap-2">
            {busy ? (
              <Button
                variant="danger"
                size="sm"
                icon={<Square className="h-3.5 w-3.5" />}
                onClick={onCancel}
              >
                Stop topology
              </Button>
            ) : (
              <Tooltip content={TOPOLOGY_HINT}>
                <Button
                  variant="primary"
                  size="sm"
                  icon={<Network className="h-3.5 w-3.5" />}
                  disabled={!credentialStatus.configured || targetCount === 0}
                  onClick={() => void onDiscover()}
                >
                  Discover topology
                </Button>
              </Tooltip>
            )}
            <p className="text-xs text-text-muted">
              {targetCount === 0
                ? "Finish a scan first. Topology uses that scan's saved inventory devices."
                : `${targetCount} device${targetCount === 1 ? "" : "s"} from this scan.`}
            </p>
          </div>
          {error ? (
            <p role="alert" className="mt-2 text-[13px] text-danger">
              {error}
            </p>
          ) : null}
        </section>

        {result ? (
          <section className="surface-panel p-4">
            <h3 className="text-[13px] font-semibold text-text">This run</h3>
            <p className="mt-1 text-[13px] text-text-secondary">{summaryLine(result.summary)}</p>
            <p className="mt-0.5 text-xs text-text-muted">
              {result.summary.devicesResponded} of {result.summary.devicesQueried} answered SNMP
              {result.summary.cancelled ? " · stopped early" : ""}
              {result.summary.timedOut ? " · hit the time budget" : ""}.
            </p>
            {result.summary.failures.length > 0 ? (
              <ul className="mt-2 space-y-1 text-xs text-text-muted">
                {result.summary.failures.slice(0, 8).map((failure) => (
                  <li key={failure.ip}>
                    <span className="mono text-text-secondary">{failure.ip}</span> · {failure.reason}
                  </li>
                ))}
              </ul>
            ) : null}

            {result.snapshot.connections.length === 0 ? (
              <p className="mt-3 text-[13px] leading-relaxed text-text-secondary">
                No neighbour relationships were proven. That usually means the devices that answered
                SNMP do not expose LLDP, CDP or a usable MAC table.
              </p>
            ) : (
              <>
                <div className="mt-3">
                  <h4 className="mb-2 text-[12px] font-semibold text-text">Preview</h4>
                  <TopologyPreview snapshot={result.snapshot} names={names} types={types} />
                </div>
              <ul className="mt-3 divide-y divide-border">
                {result.snapshot.connections.map((connection, index) => {
                  const from = endpointLabel(
                    connection.fromDeviceId,
                    connection.fromUnresolvedId ?? connection.fromLogicalId,
                    names,
                    unknownNodes,
                  );
                  const to = endpointLabel(
                    connection.toDeviceId,
                    connection.toUnresolvedId ?? connection.toLogicalId,
                    names,
                    unknownNodes,
                  );
                  const speed = speedLabel(connection.speedMbps);
                  const vlan = vlanLabel(connection);
                  return (
                    <li key={`${from}-${to}-${connection.fromPort ?? index}`} className="py-2.5">
                      <p className="text-[13px] font-medium text-text">
                        {from}
                        {connection.fromPort ? (
                          <span className="font-normal text-text-secondary">
                            {" "}
                            {connection.fromPort}
                          </span>
                        ) : null}
                        <span className="mx-1.5 text-text-muted">→</span>
                        {to}
                        {connection.toPort ? (
                          <span className="font-normal text-text-secondary"> {connection.toPort}</span>
                        ) : null}
                      </p>
                      <p className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-text-muted">
                        <Tooltip content={CONFIDENCE_HINT[connection.confidence]}>
                          <span tabIndex={0} className="inline-flex">
                            <Badge
                              tone={
                                connection.confidence === "confirmed"
                                  ? "online"
                                  : connection.confidence === "strong"
                                    ? "accent"
                                    : "warning"
                              }
                            >
                              {confidenceLabel(connection.confidence)}
                            </Badge>
                          </span>
                        </Tooltip>
                        <span>{protocolLabel(connection.protocol)}</span>
                        {speed ? <span>· {speed}</span> : null}
                        {vlan ? <span>· {vlan}</span> : null}
                        {connection.poe?.enabled ? (
                          <span>
                            · PoE
                            {connection.poe.watts != null ? ` ${connection.poe.watts} W` : ""}
                          </span>
                        ) : null}
                      </p>
                      {connection.evidence[0] ? (
                        <p className="mt-1 text-xs leading-relaxed text-text-muted">
                          {connection.evidence[0]}
                        </p>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
              </>
            )}
          </section>
        ) : null}
      </div>
    </div>
  );
}
