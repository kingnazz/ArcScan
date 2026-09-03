# ArcScan: ArcAtlas Direct Handoff

## Status

Companion implementation specification for ArcAtlas V0.5.1.

ArcScan branch: `feature/arcatlas-direct-handoff`

ArcAtlas companion branch: `feature/v0.5.1-arcscan-handoff`

ArcScan base: `4594ddc8ba10f2272804cab8679b87abd5c54e74` (v1.8.4 main).

## Goal

Let a technician explicitly send the current ArcScan Inventory snapshot directly to a site-scoped ArcAtlas Discovery Inbox without exporting a JSON file.

ArcScan remains a local scanner. ArcAtlas remains the reconciliation/source-of-truth application.

This feature does not add monitoring, background sync, an agent, or automatic documentation changes.

## Existing exporter is authoritative

Reuse the current Inventory JSON exporter in `src/lib/export.ts`.

The payload sent to ArcAtlas must use the exact row shape produced by:

`buildInventoryExport(rows, "json", notes)`

Do not create a second ArcAtlas-specific device mapper that can drift from Inventory JSON.

ArcAtlas V0.5 already parses this format and preserves its conservative presence semantics.

## User workflow

1. ArcAtlas owner/admin creates a site-scoped ArcScan connection and receives:
   - ArcAtlas server URL
   - one-time connection token
2. In ArcScan, the technician opens **ArcAtlas connection**.
3. Paste server URL and token.
4. ArcScan validates the connection.
5. Show the destination returned by ArcAtlas, for example:
   - Cedar Ridge Property Management
   - Seattle Headquarters
6. Technician runs ArcScan normally.
7. In Inventory, choose **Send to ArcAtlas**.
8. ArcScan shows the current network, device count, and destination before sending.
9. Technician confirms.
10. ArcScan sends one current-network Inventory snapshot.
11. On success, show run counts and **Open in ArcAtlas**.
12. Opening uses the existing Tauri opener/system browser.

Do not auto-send after scan completion.

## One network per send

An ArcAtlas connection is site-scoped.

Only send Inventory rows for one ArcScan network at a time.

If the Inventory UI is currently showing all networks, require the technician to choose a network before **Send to ArcAtlas** is enabled.

Never silently combine multiple ArcScan networks into one ArcAtlas site.

The displayed confirmation must clearly show which ArcScan network will be sent.

## Connection storage

The ArcAtlas token is a secret.

### Installed edition

Persist the token using the operating system credential store from Rust, not browser `localStorage`, React persistence, SQLite plaintext, or a normal config JSON file.

Preferred implementation: Rust `keyring` crate or an equivalent OS credential-store abstraction.

Windows should use Windows Credential Manager through the selected abstraction.

Persist the ArcAtlas server URL alongside/associated with the credential as needed.

The frontend may receive non-secret connection metadata but must not be able to read the stored token back after setup.

### Portable edition

Preserve ArcScan v1.8.4 disposable-session guarantees.

Portable must **not persist the ArcAtlas token** to:

- the portable executable folder
- the temporary session database
- localStorage
- a config file
- OS credential manager

Keep the token only in Rust process memory for that Portable session.

When Portable exits, the connection secret is gone.

Show concise UI copy such as:

`Portable: ArcAtlas connection is kept for this session only.`

Do not weaken the existing Portable cleanup/runtime design.

## Secret handling

The token may exist briefly in the password input while the technician pastes it.

After save/validation:

- clear the input
- do not keep plaintext in React state
- do not log it
- do not include it in frontend errors
- do not include it in exported diagnostics
- do not append it to URLs
- do not send it to any host except the configured ArcAtlas server

Use `Authorization: Bearer <token>`.

## Networking architecture

Perform ArcAtlas HTTP calls from the **Rust/Tauri backend**, not browser `fetch`.

Reasons:

- avoids CORS coupling
- keeps persisted secrets out of the WebView
- avoids widening frontend CSP `connect-src`
- gives one place to enforce URL/redirect/timeout behavior

Use a small HTTP dependency such as `reqwest` with TLS.

### URL rules

Accept normal HTTPS ArcAtlas URLs.

For development only, permit HTTP loopback hosts such as:

- `localhost`
- `127.0.0.1`
- `[::1]`

Reject other plaintext HTTP server URLs.

Reject non-http(s) schemes.

Normalize trailing slash before storing/using.

### Redirects

Do not automatically follow redirects while sending a request containing the Bearer token.

This prevents credentials from being forwarded to an unexpected host.

If the configured endpoint redirects, report a clear connection error instead.

### Timeouts

Use bounded connect/request timeouts appropriate for a <=10 MB upload.

A hung ArcAtlas server must not hang ArcScan indefinitely.

## Tauri commands / Rust service

Keep token operations behind Rust commands or a dedicated Rust module.

Suggested behavior, names may vary:

### Save/validate connection

`configure_arcatlas_connection(server_url, token)`

- validate URL
- call ArcAtlas GET `/api/discovery/arcscan`
- only persist secret after successful validation
- return minimal non-secret destination metadata

### Connection status

`get_arcatlas_connection()`

Return only:

- configured yes/no
- server URL
- connection name
- client name
- site name
- token prefix if ArcAtlas returns it
- last validation timestamp if useful
- portable session-only indicator

Never return stored plaintext token.

### Disconnect

`disconnect_arcatlas_connection()`

Installed:
- remove OS credential-store entry

Portable:
- clear in-memory secret

This does not revoke the token on ArcAtlas. UI should make that distinction clear.

### Send inventory

`send_inventory_to_arcatlas(...)`

Rust retrieves the secret internally and POSTs to:

`/api/discovery/arcscan`

The frontend passes the transport envelope/inventory data but not the persisted token.

## Transport envelope

POST JSON:

```json
{
  "schemaVersion": 1,
  "handoffId": "uuid",
  "sourceVersion": "1.8.4 or current app version",
  "generatedAt": "ISO timestamp",
  "networkName": "Current ArcScan network",
  "inventory": []
}
```

`inventory` is the parsed JSON result of the current `buildInventoryExport(..., "json")` output for the selected network.

Do not include credentials, local database paths, machine usernames, or unrelated ArcScan application state.

## Handoff id / retries

Generate one UUID for a send attempt before network transmission.

If a request times out or returns an uncertain server failure and the UI offers **Retry**, reuse the same `handoffId` for that retry so ArcAtlas can make the operation idempotent.

After a confirmed success, the next deliberate send gets a new handoff id, even if the inventory did not change.

Do not automatically loop retries in the background.

## Inventory notes

The existing Inventory JSON exporter can include ArcScan device notes.

Preserve current export behavior.

ArcAtlas V0.5 already treats imported notes as source evidence and does not automatically merge them into documented device notes.

Do not create a new note-sync behavior.

## UI

Keep ArcScan's current visual language.

Do not redesign the application.

### Inventory action

Add a clear but restrained **Send to ArcAtlas** action near the existing Inventory export controls.

States:

- Not connected
- Connected to `<client> / <site>`
- Sending
- Sent
- Retry available
- Connection rejected/revoked

If not connected, the action should open the ArcAtlas connection setup rather than failing mysteriously.

### Confirmation

Before sending show:

- destination client/site
- ArcScan network
- record count
- short explanation: `Sends observed inventory to ArcAtlas Discovery. It does not change documented devices.`

### Success

Show:

- record count
- present/missing/unknown counts returned by ArcAtlas
- destination
- **Open in ArcAtlas**

Do not call devices online/offline/down.

### Connection management

Add a compact ArcAtlas connection panel/dialog accessible without hunting through the app.

Show:

- server
- connected destination
- connection status
- Portable session-only note where relevant
- Disconnect

Do not display the stored token after successful setup.

## ArcAtlas API behavior expected

GET `/api/discovery/arcscan`

Authorization Bearer token.

Returns minimal scoped connection metadata.

POST `/api/discovery/arcscan`

Authorization Bearer token plus transport envelope.

Expected response includes:

- runId
- recordCount
- presentCount
- missingCount
- unknownCount
- clientName
- siteName
- discoveryUrl
- duplicate

Handle common statuses cleanly:

- 200 idempotent retry success
- 201 new import success
- 400 malformed request
- 401 invalid/revoked connection
- 413 payload too large
- 422 inventory/network validation error
- 5xx server failure

Do not show raw server stack traces in ArcScan UI.

## Portable and ConnectWise Backstage

The direct handoff must remain usable in the Portable build.

Portable connection setup is session-only by design, which is appropriate for a disposable onsite/Backstage run.

Do not introduce persistence merely to make the connection survive Portable restarts.

Do not add a background service or installer requirement.

## Existing behavior to preserve

Do not break:

- normal scans
- Inventory
- Changes
- scan/export JSON/CSV/XML
- manual Inventory JSON export
- SQLite inventory/history
- present/missing/unknown semantics
- installed updater
- Portable disposable state
- Portable updater exclusion

## Tests

### Frontend

- Send action requires one network scope
- confirmation shows destination/network/count
- no automatic send after scan
- success counts render
- 401 shows reconnect/reconfigure state
- timeout offers retry using same handoff id
- successful next send gets a new id
- copy never says offline/down for presence

### Rust

- HTTPS URL accepted
- localhost development HTTP accepted
- non-loopback HTTP rejected
- non-http scheme rejected
- redirects are not followed with credentials
- token never appears in public connection-status DTO
- installed credential abstraction stores/retrieves/deletes secret
- Portable credential implementation is memory-only
- disconnect clears secret
- bearer header only sent to configured ArcAtlas origin
- bounded timeout

### Export compatibility

- direct handoff uses existing `buildInventoryExport` JSON shape
- current ArcScan Inventory fixture produces expected envelope
- one network only
- notes behavior unchanged

### Regression

- existing export tests pass
- existing Inventory/Changes tests pass
- Portable runtime tests pass
- updater feature tests/config checks pass

## Quality gates

Run the repository's existing gates, including:

- `npm ci`
- `npm run typecheck`
- `npm test`
- `npm run build`
- `npm run verify-ui`
- `npm run verify-csp`
- `npm run check-version`
- `cargo test` / `cargo check` as appropriate for the current repo workflow
- Portable build/runtime checks already present in CI
- `git diff --check`

Do not weaken CSP or tests to make the feature pass.

## Versioning

Do not bump ArcScan's public release version merely to implement the feature branch unless the release workflow requires it.

Version/release naming can be decided after both cross-repository PRs are approved.

## Non-goals

Do not add:

- auto-send after scan
- background sync
- scheduled sends
- a resident ArcAtlas agent
- ArcAtlas login/password inside ArcScan
- Supabase API keys inside ArcScan
- workspace-wide credentials
- automatic reconciliation
- device creation
- documented-field updates
- SNMP
- LLDP
- UniFi/SonicWall/Synology integrations
- RMM actions
- remote commands
- vulnerability/CVE inference
- AI matching
- QR setup

## Git

Implement on:

`feature/arcatlas-direct-handoff`

Open a PR against ArcScan main.

Suggested title:

`ArcScan: Add direct ArcAtlas discovery handoff`

Do not merge until the ArcAtlas V0.5.1 receiver is reviewed and deployment order is approved.
