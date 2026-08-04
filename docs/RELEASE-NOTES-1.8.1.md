# ArcScan 1.8.1

**A cleaner window, and your public address on the screen you actually use.**

v1.8.0 added the persistent Inventory and the Changes list. v1.8.1 changes none
of that. It fixes the window showing the product name twice, moves the optional
public-IP lookup out of Settings and onto the Scan screen, and tidies the
spacing, alignment and contrast around both.

Install over v1.8.x, v1.7.x or v1.6.x without losing anything. There is no
database change in this release: every scan, device, name, note, status, network
and date carries over untouched.

---

## The duplicate title is gone

The packaged app used to show ArcScan twice, stacked: once in the title bar the
operating system draws, and again immediately below it in the app's own toolbar,
which drew a second icon and wordmark of its own.

The in-app one is the one that went. The native title bar stays, because it is
the one Windows and macOS already position, theme, and hand to screen readers
and window-management shortcuts — and replacing it would mean re-implementing
minimize, maximize, restore, snapping, full screen and keyboard window controls
on both platforms for no gain.

With the brand block gone, the view switcher takes the start of the row, so
`Scan · Inventory · Changes · History` is now the first thing in the window.
Nothing was left behind in its place: there is no blank strip where it used to
be.

This was invisible in development and in every automated check, because a
browser has no native title bar to duplicate. The browser suite now asserts that
the app renders no title or icon of its own, so it cannot come back.

## Public IP on the Scan screen

The lookup already existed, at the bottom of Settings, which is not where anyone
looks for the address their connection appears from. It now sits in a compact
row directly under the view switcher, beside the local network the scan will run
against:

```text
Local network  192.168.1.0/24            Public IP  Not checked  [Check]
```

Once checked, it shows the address, how long ago it was checked, and buttons to
copy it or check again. If no provider answers, it says so, offers Retry, and
keeps the raw provider errors behind **Technical details** rather than either
hiding them or shoving them at you.

### It still never runs on its own

This is the part that matters, and it has not changed:

- Nothing is looked up at startup, when the Scan screen opens, after a scan, when
  you switch views, on a timer, or in the background.
- The only thing that contacts a provider is a press on **Check**, **Refresh** or
  **Retry**.
- The providers are `api64.ipify.org`, then `icanhazip.com` if the first does not
  answer. They are asked for your address and told nothing else. Your targets,
  results, device names, MAC addresses and notes are never sent anywhere, to them
  or to anybody.
- The answer is held in memory for the session. It is not written to the
  database, not written to your preferences, and not included in any export.
  Closing ArcScan forgets it.
- Settings keeps the full explanation, shows the session's value, and can forget
  it on demand.

Settings' switch now decides whether the utility is offered at all, which is what
it always did — it never controlled whether a request happened by itself, because
nothing ever did. It is on for new installs. **If you explicitly turned it off in
an earlier version, it stays off**, and the Scan screen simply does not show the
utility until you turn it back on.

## Visual refinements

Restrained, and deliberately not a redesign:

- The header is navigation and window actions, with the tab counts on a shared
  badge that no longer shifts the tabs sideways as the numbers grow.
- A context row for the local network and the public-IP utility, on the Scan
  screen only — Inventory, Changes and History describe stored data rather than
  the current connection, so they do not pay the vertical space for it.
- Consistent control heights and spacing across the scan controls and the search
  toolbar.
- Two contrast fixes that predate this release: the tab count badges and, more
  visibly, the **Stop** button, which sat at 4.3:1 against its own hover tint for
  as long as a scan was running. Both now clear WCAG AA on every surface they
  appear on, in both themes.
- Two controls in the results toolbar were both called "Clear the filter". They
  are now "Clear the search" and "Clear all filters".

## What has not changed

No scanner semantics, no partial-scan handling, no Inventory presence rules, no
network scoping, no Changes persistence, no migrations and no export formats.
Scan, Inventory, Changes and History are the same four views doing the same
things. This release touches how ArcScan looks and where the public-IP lookup
lives, and nothing underneath it.

## Verified

- 218 TypeScript unit tests and 159 Rust tests.
- 56 end-to-end browser checks, including every Public IP state driven through
  scripted providers: first-provider failure and fallback, total failure, retry,
  a slow lookup, repeated presses producing one request, and the address
  surviving a view change and a scan while being written to no storage.
- 26 axe-core sweeps across every view, all four Public IP states, Settings, a
  scan in flight and the 940px minimum width, in both themes, with no violations.
- The production Content Security Policy exercised against the real built assets,
  plus static assertions that it allows no wildcard or plaintext origins, no
  `unsafe-eval`, and exactly the two declared providers and nothing else.

### Not verified here

Native window behaviour cannot be exercised in a browser. The single title and
icon, dragging, double-click maximize, snapping, full screen, the macOS traffic
lights and high-DPI scaling all need a packaged build on a real Windows and
macOS machine. The checklist is in
[docs/WINDOW-CHROME.md](WINDOW-CHROME.md).

The real providers are likewise only contacted by the packaged app: the browser
demo answers from scripted ones so it never makes an outbound request and never
shows a real address. The fallback logic is the same code in both.
