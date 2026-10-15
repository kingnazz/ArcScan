# ArcScan interface

How the ArcScan desktop interface is put together, and why. Written for the
1.10 interface pass; update it when the shell or a shared pattern changes.

## 1. Audit of the 1.9.1 interface

What was found before anything was changed. Each finding names the surface it
came from.

### Visual hierarchy

- The Scan screen stacked four full-width bands before any content: view
  switcher, context row, command bar, results toolbar. The detected network was
  printed three times on the start screen (context row, start card, target
  field).
- The command bar read `Quick LAN | Quick | 192.168.1.0/24 | Scan`. The thing a
  technician must confirm first — which network — was the third control, and
  two identical dropdowns reading "Quick LAN" and "Quick" sat in front of it.
- The workload preview (addresses · ports) was hidden below 1280px and sat at
  the far right, away from the field it describes.
- The active view in the header was a raised segment on a sunken track, which
  is close to invisible in the dark theme.

### Controls competing for attention

- "Advanced" was a bordered button with the same weight as the profile picker.
- The results toolbar mixed filters (Labelled, Changes only), view switches
  (Topology), a comparison entry disguised as an uppercase badge ("NO
  CHANGES"), Export, and an unlabelled × that cleared filters.
- Every Changes entry carried four to six buttons; seven entries meant forty
  buttons on screen, all at similar weight.

### Too dense / strangely empty

- The device drawer opened on two form fields and fourteen flat detail rows;
  discovery evidence then ran on for another screen.
- The start screen was a centred web-page card with four paragraphs of
  guidance and a large empty canvas around it.
- History rows were built of five or six uppercase pills each.

### Navigation

- The top-level switcher (Scan / Inventory / Changes / History) is the right
  structure: four peers, always one click away. It was kept.
- The scan command bar appeared on Inventory, Changes and History, where the
  user is reviewing stored data. It cost ~50px on each and competed with the
  view's own actions.
- The scan's own comparison was reachable only through a badge; Topology was a
  toggle button in the filter toolbar.
- Nothing outside the Scan view showed that a scan was running.

### Inconsistency

- Three separate export popovers (w-44, w-52, w-28; two without menu roles).
- Search fields at 13rem and 14rem, clear buttons at two sizes.
- Uppercase 10.5px badges used for everything from network names to evidence
  freshness to "Answered this scan".
- Version printed twice (header and status bar).

### Terminology

- "Mark unreviewed" (a device classification) collided with "Unreviewed" (a
  change state).
- "Labelled" filter vs "Known / Trusted / Watched" statuses.
- "State" (scan table) vs "Status" (inventory presence) vs "Status" (drawer
  classification) for three different things.
- "Back" (table badge) vs "Returned" (everywhere else).
- "Why no comparison?" as a warning-coloured badge.

### Actions in unexpected places

- Send to ArcAtlas lived only in the Inventory toolbar, visible before any scan
  had run, with no route from a completed scan.
- The ArcAtlas connection could not be managed anywhere once connected (the
  handler was passed to Inventory but never rendered).
- The "Use this computer's network" crosshair and Rescan were unlabelled icons
  beside Scan.

### Empty and scan states

- Never scanned: a card duplicating the command bar, plus a wall of guidance.
- Scanning: a 2px strip and a status-bar sentence at the bottom of the window.
  Nothing near the table said what phase the scan was in or that Stop keeps
  what was found.
- Finished: no completion moment. The only signals were the status bar and an
  uppercase badge.
- Stopped: the status bar said "stopped after N of M"; nothing in the results
  area explained that a partial scan is saved but never compared.
- No hosts: a generic empty state.

### Drawer and dialog usability

- Actions wrapped to two rows in the footer, with three disabled ghosts
  (Shares, RDP, SSH) at the same size as enabled ones.
- Section heading "IDENTITY" immediately followed by field label "NAME": two
  uppercase labels stacked.
- Presence, classification and change were mixed into one "State" row.

### Technical detail dominating

- Discovery evidence (sources, freshness, identity keys, advertised services,
  stale rows, alternate names) was presented at the same level as the device's
  name and address.
- Settings was one 830-line scroll: appearance, ports, concurrency, column
  toggles, retention, network naming, discovery, credentials, privacy, about.

### A collection of features rather than one product

- Inventory, Changes and History each had a bare filter row with no statement of
  what they are for, so they read as three unrelated mini-apps.
- ArcAtlas, Topology and the comparison each arrived in a different corner with
  a different control style.

## 2. Design principles

1. **One question per region.** Header: where am I. Scan bar: what will be
   scanned, how deeply, go. Status line: what is happening / what happened.
   Toolbar: how am I looking at it. Table: the facts.
2. **Read order is task order.** Target → profile → depth → Scan.
3. **The same line narrates the whole scan.** The scan bar's second line is the
   pre-flight summary before a scan, progress during it and the outcome after
   it, so empty → scanning → results is one continuous surface rather than
   three screens.
4. **Primary facts first, evidence on request.** Name, address, type and
   services are always visible; how ArcScan knows is one click away and never
   hidden.
5. **Quiet chrome, loud data.** Borders and type weight do the work; colour is
   reserved for state (answered, new, changed, missing, warning) and for the
   one primary action.
6. **Every control keeps its place.** Unavailable actions are disabled with a
   reason, not removed, so the layout never jumps.

## 3. The shell

```
┌ header ───────────────────────────────────────────────────────────────────┐
│ Scan●  Inventory 21  Changes 7  History              ⤓  ☀  ⚙              │
├ scan bar (Scan view only) ────────────────────────────────────────────────┤
│ [192.168.1.0/24            ▾] [Profile Quick LAN ▾] [Depth Quick ▾] [▶ Scan] ⟳ Advanced
│ ● This computer's network · Wi‑Fi · 254 addresses · 14 ports    Public IP │
├ toolbar ──────────────────────────────────────────────────────────────────┤
│ Devices 12 | Comparison 3 | Topology   [Filter devices]  …  Export  ArcAtlas
├ table ────────────────────────────────────────────────────────────────────┤
```

- **Header.** Tabs with an accent underline for the active view. The Scan tab
  carries a live indicator while a scan runs, so a scan is visible from every
  view. Utilities (update check, theme, settings) are quiet icons on the right.
  The version lives in the status bar only.
- **Scan bar.** Only on the Scan view. Its second line is the scan status line
  (below). Public IP stays on the right of that line.
- **View header.** Inventory, Changes and History open with one line stating
  what the view is for, a count, and the view's primary actions. This is where
  the three views explain their relationship:
  - Inventory — *What ArcScan believes exists, across every scan.*
  - Changes — *What changed between scans, and what still needs a look.*
  - History — *Every scan that ran, and what each one found.*
- **Status bar.** Scan counts and timing, edition, version.

## 4. Scan states

| State | Status line | Results area |
| --- | --- | --- |
| Never scanned | Pre-flight: network, workload | Start panel: detected network, one Scan action, other networks, recents |
| Ready (edited target) | Pre-flight, updated as you type | Previous results, or start panel |
| Validating | Pre-flight hides invalid workload | Error toast names the problem on Scan |
| Scanning — finding hosts | Step 1 of 3, determinate bar, n of m checked, found | Rows stream in; placeholder row until the first reply |
| Scanning — identifying | Step 2 of 3, indeterminate | Pending cells show a resolving mark |
| Scanning — resolving names | Step 3 of 3 | — |
| Stopping | "Stopping — keeping what was found" | — |
| Stopped manually | Warning tone: partial, saved, not compared | Rows kept |
| Complete | Success: n devices, time, comparison summary | Next steps in the toolbar: comparison, export, ArcAtlas |
| Complete with warnings | Warning notice under the bar | Rows |
| No hosts found | Neutral: nothing answered | Empty state with the Reliable LAN suggestion |
| Saved scan reopened | "Saved scan from …" | Rows |

## 5. Patterns

- **Chips** replace uppercase badges: sentence case, 11px, a tinted dot or icon
  for state. Uppercase is kept only for the small section labels.
- **Export menu** is one component used by Scan, Inventory, Changes and each
  History row.
- **Search field** is one component with one width and one clear button.
- **Tables**: sentence-case headers, hairline rows, no zebra striping, a 2px
  accent bar and tint on the selected row, sort arrows on hover.
- **Inspector sections**: a small label, a two-column fact grid, and an
  *Evidence* disclosure for how ArcScan knows.
- **Settings** is grouped into Scanning, Discovery, Credentials, Networks,
  Appearance and Application, with a rail that jumps between them.
