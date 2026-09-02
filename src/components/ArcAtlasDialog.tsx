import { useEffect, useState } from "react";
import { Button, Field, FieldRow } from "../ui/primitives";
import {
  DISCONNECT_WARNING,
  PORTABLE_SESSION_COPY,
  destinationLabel,
  displayTokenPrefix,
  type ArcAtlasConnection,
  type ArcAtlasError,
  type ArcAtlasSendResult,
  type SendConfirmation,
  successCounts,
} from "../lib/arcatlas";

export type ArcAtlasDialogMode = "connect" | "status" | "confirm" | "success" | "error";

export interface ArcAtlasDialogProps {
  open: boolean;
  mode: ArcAtlasDialogMode;
  connection: ArcAtlasConnection;
  confirmation?: SendConfirmation | null;
  result?: ArcAtlasSendResult | null;
  error?: ArcAtlasError | null;
  busy?: boolean;
  onClose: () => void;
  onConfigure: (serverUrl: string, token: string) => Promise<void> | void;
  onDisconnect: () => Promise<void> | void;
  onReconnect: () => void;
  onSend: () => Promise<void> | void;
  onRetry: () => Promise<void> | void;
  onOpenInArcAtlas: (url: string) => void;
}

export function ArcAtlasDialog(props: ArcAtlasDialogProps) {
  const [serverUrl, setServerUrl] = useState(props.connection.serverUrl ?? "");
  const [token, setToken] = useState("");

  useEffect(() => {
    if (props.open && props.mode === "connect") {
      setServerUrl(props.connection.serverUrl ?? "");
      setToken("");
    }
  }, [props.open, props.mode, props.connection.serverUrl]);

  if (!props.open) return null;

  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center p-4">
      <div className="animate-fade-in absolute inset-0 bg-black/40" onClick={props.onClose} aria-hidden />
      <div role="dialog" aria-modal="true" aria-labelledby="arcatlas-title" className="popover animate-slide-up relative w-full max-w-md p-4">
        {props.mode === "connect" ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void (async () => {
                await props.onConfigure(serverUrl, token);
                setToken("");
              })();
            }}
          >
            <h2 id="arcatlas-title" className="text-sm font-semibold text-text">
              {props.connection.needsReconfigure || props.error?.code === "unauthorized"
                ? "Reconnect ArcAtlas"
                : "Connect ArcAtlas"}
            </h2>
            <p className="mt-1.5 text-[13px] leading-relaxed text-text-secondary">
              Paste the ArcAtlas server URL and connection token. The token is stored only after ArcAtlas accepts it.
            </p>
            <div className="mt-3 space-y-3">
              <FieldRow label="ArcAtlas server URL" htmlFor="arcatlas-server">
                <Field
                  id="arcatlas-server"
                  value={serverUrl}
                  onChange={(event) => setServerUrl(event.target.value)}
                  placeholder="https://atlas.example.com"
                  autoComplete="off"
                />
              </FieldRow>
              <FieldRow label="Connection token" htmlFor="arcatlas-token">
                <Field
                  id="arcatlas-token"
                  type="password"
                  value={token}
                  onChange={(event) => setToken(event.target.value)}
                  autoComplete="off"
                />
              </FieldRow>
            </div>
            {props.connection.portableSessionOnly ? (
              <p className="mt-2 text-xs text-text-muted">{PORTABLE_SESSION_COPY}</p>
            ) : null}
            {props.error ? (
              <p className="mt-2 text-xs text-danger" role="alert">
                {props.error.message}
              </p>
            ) : null}
            <div className="mt-4 flex justify-end gap-2">
              <Button type="button" onClick={props.onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={props.busy || !serverUrl.trim() || !token.trim()}>
                {props.busy ? "Connecting" : "Connect"}
              </Button>
            </div>
          </form>
        ) : null}
        {props.mode === "status" ? (
          <div>
            <h2 id="arcatlas-title" className="text-sm font-semibold text-text">
              ArcAtlas
            </h2>
            <p className="mt-1 text-[13px] text-text-secondary">{props.connection.configured ? "Connected" : "Not connected"}</p>
            {props.connection.serverUrl ? (
              <p className="mt-2 text-[13px] text-text">
                <span className="text-text-muted">Server: </span>
                {props.connection.serverUrl}
              </p>
            ) : null}
            {props.connection.configured ? (
              <p className="text-[13px] text-text">
                <span className="text-text-muted">Destination: </span>
                {destinationLabel(props.connection)}
              </p>
            ) : null}
            {displayTokenPrefix(props.connection.tokenPrefix) ? (
              <p className="text-[13px] text-text">
                <span className="text-text-muted">Token: </span>
                {displayTokenPrefix(props.connection.tokenPrefix)}
              </p>
            ) : null}
            {props.connection.portableSessionOnly ? (
              <p className="mt-2 text-xs text-text-muted">{PORTABLE_SESSION_COPY}</p>
            ) : null}
            <p className="mt-2 text-xs leading-relaxed text-text-muted">{DISCONNECT_WARNING}</p>
            <div className="mt-4 flex justify-end gap-2">
              <Button type="button" onClick={props.onClose}>
                Close
              </Button>
              <Button type="button" onClick={props.onReconnect}>
                Reconnect
              </Button>
              <Button type="button" variant="danger" onClick={() => void props.onDisconnect()}>
                Disconnect
              </Button>
            </div>
          </div>
        ) : null}
        {props.mode === "confirm" && props.confirmation ? (
          <div>
            <h2 id="arcatlas-title" className="text-sm font-semibold text-text">
              Send to ArcAtlas
            </h2>
            <dl className="mt-3 space-y-1.5 text-[13px]">
              <div>
                <dt className="text-text-muted">Destination</dt>
                <dd className="text-text">{props.confirmation.destination}</dd>
              </div>
              <div>
                <dt className="text-text-muted">Network</dt>
                <dd className="text-text">{props.confirmation.networkName}</dd>
              </div>
              <div>
                <dt className="text-text-muted">Devices</dt>
                <dd className="text-text">{props.confirmation.deviceCount}</dd>
              </div>
            </dl>
            <p className="mt-3 text-[13px] leading-relaxed text-text-secondary">{props.confirmation.explanation}</p>
            <div className="mt-4 flex justify-end gap-2">
              <Button type="button" onClick={props.onClose}>
                Cancel
              </Button>
              <Button type="button" variant="primary" disabled={props.busy} onClick={() => void props.onSend()}>
                {props.busy ? "Sending" : "Send to ArcAtlas"}
              </Button>
            </div>
          </div>
        ) : null}
        {props.mode === "success" && props.result ? (
          <Success result={props.result} onClose={props.onClose} onOpen={props.onOpenInArcAtlas} />
        ) : null}
        {props.mode === "error" && props.error ? (
          <div>
            <h2 id="arcatlas-title" className="text-sm font-semibold text-text">
              {props.error.code === "unauthorized" ? "Reconnect ArcAtlas" : "Could not send to ArcAtlas"}
            </h2>
            <p className="mt-1.5 text-[13px] leading-relaxed text-text-secondary">{props.error.message}</p>
            <div className="mt-4 flex justify-end gap-2">
              <Button type="button" onClick={props.onClose}>
                Close
              </Button>
              {props.error.code === "unauthorized" ? (
                <Button type="button" variant="primary" onClick={props.onReconnect}>
                  Reconfigure
                </Button>
              ) : props.error.retryable ? (
                <Button type="button" variant="primary" onClick={() => void props.onRetry()}>
                  Retry
                </Button>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function Success(props: { result: ArcAtlasSendResult; onClose: () => void; onOpen: (url: string) => void }) {
  const counts = successCounts(props.result);
  return (
    <div>
      <h2 id="arcatlas-title" className="text-sm font-semibold text-text">
        Sent to ArcAtlas
      </h2>
      <ul className="mt-3 space-y-1 text-[13px] text-text">
        <li>Observed: {counts.observed}</li>
        <li>Present: {counts.present}</li>
        <li>Not observed: {counts.notObserved}</li>
        <li>Unknown: {counts.unknown}</li>
      </ul>
      <p className="mt-2 text-[13px] text-text-secondary">
        {destinationLabel({ clientName: props.result.clientName, siteName: props.result.siteName })}
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <Button type="button" onClick={props.onClose}>
          Close
        </Button>
        <Button type="button" variant="primary" onClick={() => props.onOpen(props.result.discoveryUrl)}>
          Open in ArcAtlas
        </Button>
      </div>
    </div>
  );
}
