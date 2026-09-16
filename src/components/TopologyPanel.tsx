// Topology discovery: SNMP credentials, a run button, a concise result.
//
// This is a Deep Scan / credentialed step. Quick LAN does not run it. The
// panel is deliberately small — ArcAtlas owns the map.

import { useState } from "react";
import { Network, Square } from "lucide-react";
import { Badge, Button, Field, FieldRow, Select } from "../ui/primitives";
import {
  SNMP_AUTH_PROTOCOLS,
  SNMP_PRIV_PROTOCOLS,
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
  type TopologyResult,
  type UnresolvedNode,
} from "../lib/topology";

export interface TopologyPanelProps {
  credentialStatus: CredentialStatus;
  result: TopologyResult | null;
  names: DeviceNameLookup;
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
      <div className="mx-auto max-w-3xl space-y-4 px-4 py-4">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold text-text">Topology discovery</h2>
            <p className="mt-1 text-[13px] leading-relaxed text-text-secondary">
              Credentialed SNMP walk of the current devices. Quick LAN does not run this. Links
              stay on this computer; credentials are never exported.
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
              <Button
                variant="primary"
                size="sm"
                icon={<Network className="h-3.5 w-3.5" />}
                disabled={!credentialStatus.configured || targetCount === 0}
                title={
                  !credentialStatus.configured
                    ? "Enter SNMP credentials first"
                    : targetCount === 0
                      ? "Scan a network first"
                      : `Query ${targetCount} device${targetCount === 1 ? "" : "s"} over SNMP`
                }
                onClick={() => void onDiscover()}
              >
                Discover topology
              </Button>
            )}
            <p className="text-xs text-text-muted">
              {targetCount === 0
                ? "Scan a network first. Topology uses the devices from this scan."
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
              <ul className="mt-3 divide-y divide-border">
                {result.snapshot.connections.map((connection, index) => {
                  const from = endpointLabel(
                    connection.fromDeviceId,
                    connection.fromUnresolvedId,
                    names,
                    unknownNodes,
                  );
                  const to = endpointLabel(
                    connection.toDeviceId,
                    connection.toUnresolvedId,
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
            )}
          </section>
        ) : null}
      </div>
    </div>
  );
}
