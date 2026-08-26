# ArcScan 1.8.3

**Correct what it detected, and see when an answer has gone stale.**

v1.8.2 started asking the local network what its devices are, over mDNS and
SSDP. It was right most of the time and wrong some of the time, and there was
nothing a person could do about either case. v1.8.3 is about what happens after
that first answer: you can correct it, ArcScan tells you when the evidence
behind it has gone quiet, and it can hand you a short redacted summary when it
gets one wrong.

This is a refinement release. No new protocol, no new probe, no new outbound
request, and no new primary view. Everything below is built from evidence
v1.8.2 already collected.

Install over 1.8.x, 1.7.x or 1.6.x without losing anything. Every scan, device,
name, note, status, network, date and piece of discovery evidence is kept, and
the database migrates in place.

---

## What is new

### A device type you can set yourself

Every device now has a device type you can choose. The device panel offers the
same fourteen types v1.8.2 shipped, plus **Automatic**, which is the default and
means "use whatever ArcScan works out".

Your choice wins, everywhere: in the Inventory's Type column, in the type
filter, in search, and in every export.

**The correction never changes what the device *is*.** It changes what ArcScan
calls it, and that is all. The device keeps:

- the same `identity_key` and the same `identity_source`
- the same MAC address, and the same network scope
- the same first-seen and last-seen dates
- the same presence state, trust status, name and notes
- the same observation history and the same change events

Nothing is merged, nothing is split, and no duplicate device appears. In the
database it is one nullable column on one row, written by a command that touches
nothing else.

**What ArcScan detected is kept underneath rather than replaced.** The panel
shows both, so you can see what you overruled and change your mind later.
Choosing Automatic again reveals whatever ArcScan currently thinks, which may
have moved on since you corrected it — not a snapshot of what it thought when
you did.

**Unknown is a real answer.** Choosing it is not the same as leaving the type on
Automatic. It is a person saying "ArcScan is wrong and I cannot say either",
which is a genuine conclusion, and the next scan does not talk them out of it.
The two states are stored differently and behave differently.

**A correction is not a network event.** Setting, changing or clearing one
records nothing in the Changes inbox. It is an edit you made to ArcScan, not
something that happened on your network, and putting it in the review inbox
would be noise.

There are no bulk type corrections in this release.

### Evidence that goes quiet stops being authoritative

ArcScan is not a monitor. It learns when you run a scan, and in v1.8.2 an answer
it heard once stayed authoritative forever, however long ago that was.

Discovery evidence now carries one of three states:

| State | Meaning |
| --- | --- |
| **Current** | Re-observed by the most recent qualifying scan. |
| **Getting old** | Missed by one or two qualifying scans. Still believed. |
| **Stale** | Missed by three consecutive qualifying scans. Kept and shown, but no longer enough on its own. |

**A qualifying scan is a strict thing.** A miss is only counted when *all* of
these were true:

- the scan completed and was not stopped;
- it ran both discovery protocols to completion;
- the device was found by that scan;
- the protocol that would have carried the evidence actually ran. In particular,
  the manufacturer, model and serial only ever come from a device description
  document, so they only age when a description was actually read.

Nothing ages because a scan was partial, was of a remote network, had discovery
switched off, had no eligible local interface, or simply did not find the
device. A device that was unplugged has not stopped advertising.

**It is counted in scans, not in days.** A wall-clock age would say more about
when the operator last ran ArcScan than about the device, so the panel says
`last seen 4 discovery scans ago` rather than a date. The threshold is a code
constant (`STALE_AFTER_MISSES = 3`), deliberately not a setting: the number only
means anything alongside the definition of a qualifying miss, and exposing one
without the other invites someone to turn it down to 1 and then disbelieve the
result.

**What stale evidence does and does not do.** It stays stored, stays visible in
the device panel, keeps its first-seen and last-seen dates, and stays in the
diagnostic report. It is never deleted automatically. It has no effect at all on
a type you set yourself.

What it loses is the ability to carry a High-confidence claim on its own. A
device type resting entirely on stale evidence is shown at Medium rather than
High, and the panel says why. It does not drop further, and the device does not
become something else: the evidence was real, and a printer that has stopped
advertising is still overwhelmingly likely to be a printer. One scan that hears
it again puts it straight back.

That reduction happens at read time and is never written to the database, which
is why aging alone produces no change events and needs no scan to undo.

### Classification, tuned against the same evidence

No new protocols and no external lookups. Every rule below reads what v1.8.2
already collected more carefully:

- A **television with casting built in** is told apart from the **streaming
  stick** plugged into it.
- An **Apple TV** is recognised as a media device rather than filed as a speaker
  for accepting AirPlay audio, and a **television** that accepts the same audio
  is still a television.
- **Roku** devices are recognised, and a Roku television reads as a television.
- A **named speaker** streaming audio is recognised at High confidence.
- A **NAS** that shares files, comes from a storage maker, and serves its own
  web console or media server reaches High confidence.
- A **gateway** that advertised only `WANDevice` or `WANConnectionDevice`, rather
  than the whole `InternetGatewayDevice`, is still recognised as a router.
- An **access point, switch or controller** is recognised as network equipment —
  unless it is also this network's gateway, in which case Router is the more
  useful answer and wins.
- **Two smart-home protocols** together (HomeKit and Matter) reach High.
- A **desktop** sharing its screen alongside a shell is recognised as a computer.
- **Both printing ports** open together is Medium; one alone is still Low.

**Unknown is still a valid result and still preferred to a guess.** A camera is
still never High confidence from RTSP alone, whatever else agrees with it.

### Names that read like names

- A `friendlyName` or mDNS label that is really a machine identifier — `uuid:…`,
  `urn:…`, or a bare UUID with or without its dashes — is refused outright.
  Nothing is worse to show than the address it replaced.
- A service-instance suffix is trimmed: `Office Printer._ipp._tcp.local` reads
  as `Office Printer`. Only a real service type is removed, so a name that
  merely contains an underscore keeps it.
- A factory label with a separator and a hex tail (`HP-A1B2C3`, `Canon_1A2B3C4D`)
  is demoted below the reverse-DNS hostname rather than discarded. On a shelf of
  three identical printers it is the only thing telling them apart.
- A manufacturer the model already mentions anywhere is no longer repeated, not
  only when it is the prefix.

**Names that genuinely contain digits are left exactly as they are.**
`Synology DS923+`, `HP LaserJet M404` and `Bravia KD-55X80J` all survive
untouched. A run-together factory label like `BRW90E2BA` is deliberately *not*
recognised: the same rule reads `RT2600AC`, a real Synology router model, as a
serial, and demoting a real model number is the worse mistake.

Name resolution remains deterministic and independent of packet order, now
proved over every permutation of a five-claim fixture rather than over one swap.

### Discovery quality in History

Each scan's History line now says how much its discovery pass actually managed:

```
Discovery: Complete · 12 mDNS · 8 SSDP
Discovery: Limited · mDNS socket unavailable
Discovery: Skipped · Remote subnet
Discovery: Interrupted · Scan stopped
```

- **Complete** — both protocols ran and finished, nothing was cut short. The
  line shows the counts it heard.
- **Limited** — discovery ran but could not do all of it, and the line names the
  one thing ArcScan observed.
- **Skipped** — it did not run: a remote target, switched off, or no eligible
  local interface.
- **Interrupted** — Stop landed while discovery was in progress.

**ArcScan never says a firewall blocked anything**, because it cannot observe a
firewall. It reports a socket that would not open, a response cap reached, or a
description it refused — each of which it did observe — and leaves the diagnosis
to the person, who can see their own firewall.

This is separate from the existing `discovery_mode`, which gates whether two
scans may be compared and whose meaning has not moved. A scan that could listen
is still never compared with one that could not.

### Copy discovery details

A new action in the device panel puts a short summary on the clipboard:

```
ArcScan discovery report
Version: 1.8.3
Device type: Media device
Type source: Automatic
Detected confidence: Medium
Detected name: Living Room TV
Manufacturer: Example Corp
Model: TV-123
Address: 192.168.x.x
Sources: mdns, ssdp
Discovery scan state: Complete
Services:
- _airplay._tcp
Fresh evidence:
- mdns service: _airplay._tcp
Stale evidence:
- ssdp service: MediaServer (last seen 4 discovery scans ago)
```

It exists because a classification rule cannot be fixed from a screenshot of a
device list, but it can be fixed from the services and model strings a device
advertises.

**What it never contains:** your notes, the MAC address, the serial number, the
device's UPnP UDN or mDNS instance name, any URL it advertises, its IPv6
addresses, the network's friendly name, or any database id. The local address
appears masked to its first two octets (`192.168.x.x`), which says what kind of
network without saying which device.

That is enforced twice over. The input the builder accepts has no field for any
of it, so a caller cannot pass one in by accident; and the builder drops
identifier-bearing evidence kinds regardless of what it is handed. In the
packaged app the report is built in Rust straight from the database, by a query
that selects no note, no MAC and no serial — there is nothing there to leak.

The output is deterministic, bounded to 4,000 characters, and every line is
capped, so a device advertising a megabyte of text produces a pasteable report
rather than a wall of it. Nothing is uploaded, no file is written, no request is
made, and the copy is confirmed rather than silent.

## A quiet upgrade

v1.8.3 reads some advertised names more tidily than v1.8.2 did. Left alone, the
first scan after upgrading would have reported a renamed device for every device
whose name it improved — an inbox full of events for nothing that happened on
the network.

Each stored discovery record now carries the generation of the naming rules that
wrote it. A record older than the current generation has its detected name and
its model compared **silently, exactly once**; the record that scan writes
carries the current generation, so every scan after it compares normally. The
tidied name is still adopted — only the event is suppressed.

Everything else about that first scan is compared exactly as usual.

## Nothing about identity changed

Device identity is still MAC address, then hostname-and-vendor, then address,
scoped to a network. Discovery is still evidence attached to a device, never a
key. A type correction is an operator label, alongside the name, the status and
the notes, and nothing about how devices are matched, scoped or compared reads
it.

No device is re-keyed, merged or split by anything in this release.

## Migration

Schema 5 to 6. Two nullable columns, and nothing else:

- `devices.user_device_type` — the operator's correction. `NULL` is Automatic;
  the string `unknown` is an explicit choice and is stored as one.
- `device_discovery.naming_rules_version` — which generation of the naming rules
  wrote the record. Defaults to 0, which is exactly right for every row v1.8.2
  wrote.

No table is rebuilt, no row is re-keyed, no device is reclassified, and nothing
is backfilled. The migration is transactional and idempotent: an interrupted
upgrade leaves a database that is either before or after, and opening an
already-current database changes nothing. Existing evidence starts at zero
misses, so nothing arrives already stale, and no device arrives already
corrected — the upgrade makes no choices for the operator.

## Exports

The Inventory export gains four columns, so a type in a spreadsheet can be
attributed:

| Column | Meaning |
| --- | --- |
| `Device type` | The type shown: the correction if there is one, ArcScan's answer otherwise. |
| `Type source` | `User` or `Automatic`. |
| `Detected type` | What ArcScan detected, blank where no discovery-capable scan reached the device. |
| `Detected confidence` | The detected confidence, already reduced where its evidence is stale. |
| `Discovery freshness` | `current`, `aging` or `stale`. |

Stale evidence rows themselves are not written into CSV or XML: the device panel
shows them, and one long-lived device should not dominate an export.

## Security and privacy

Unchanged from v1.8.2, and asserted by the same tests:

- mDNS is still a one-shot querier on an ephemeral port. Port 5353 is never
  bound, no query is ever answered, and nothing is kept between scans.
- The SSDP `LOCATION` rules are unchanged: plain local HTTP only, inside the
  network the scan actually ran against, no redirect, no proxy, no second name
  lookup between the check and the connect.
- Description documents still allow no DTD, and every bound on size, nesting,
  field length and element count is unchanged.
- The Content Security Policy and the webview capabilities are unchanged. No
  new permission was needed.
- No external API, no lookup service, no fingerprint database, no analytics and
  no telemetry. The new diagnostic report is built locally and goes on the
  clipboard.
- Every device-supplied string is still bounded, stripped of control characters
  and rendered as text.
- The type override is validated against the shipped vocabulary with a strict
  parser before it reaches the database. A value that is not a type is an error,
  not a silent Unknown — because Unknown is itself a meaningful answer that a
  typo must not be able to impersonate.

## Not in this release

No scheduled scans, background scanning, notifications, tray mode or launch at
login. No SNMP, no credential storage, no IPv6 scanning, no general UDP
scanning, no packet capture. No vulnerability, exploit or default-password
checks. No cloud account, sync, remote agent, collaboration or ticketing. No
topology map, no automatic remediation, no AI or cloud classification, no
external manufacturer lookup. No code signing and no macOS notarization.

No new discovery protocols, and no bulk type corrections.

## Performance

Measured against a database at the scale this release was tested for: 5,000
devices, 100,000 observations, 50,000 discovery evidence rows across 20 scans,
1,000 devices carrying a type correction and 1,000 whose evidence has gone
stale. The numbers below are from that fixture, which the Rust suite builds and
re-measures on every run.

| Operation | Time |
| --- | --- |
| Load the whole Inventory (5,000 rows) | ~4.4 s |
| Open a device panel | ~2 ms |
| Build a diagnostic report | <1 ms |
| Set one type correction | ~1 ms |

The freshness state is computed as **one grouped pass over the evidence table**,
not a query per row: measured on its own it accounts for about 42 ms, roughly 1%
of the Inventory load. The confidence reduction for stale evidence is applied to
values already in hand, so it costs nothing extra. No new index was added,
because measurement showed none was needed for the new work.

**The remaining time is the v1.8.2 Inventory query and is unchanged by this
release.** The same query with the v1.8.3 aggregate removed measures ~4.3 s on
the same fixture: the cost is the window function that picks each device's most
recent observation out of 100,000 rows, not anything added here. Making that
faster would mean changing the shape of a query this release otherwise does not
touch, so it is left for a release that can measure it properly. In practice a
5,000-device inventory is far beyond a typical home or small-office network; a
few hundred devices loads in a small fraction of this.

Nothing is reclassified at startup, no summary is rewritten when it has not
changed, and stale evidence is capped in the interface rather than loaded
without bound.

## Known limitations

- A correction applies to one device at a time. Correcting fifty devices means
  fifty edits.
- Aging only advances when a scan runs. A person who scans once a month will
  reach stale in three months; that is the intended behaviour, not a bug, and it
  is why the interface counts scans rather than days.
- The stale threshold is not configurable.
- The confidence reduction for stale evidence caps High at Medium and stops
  there. A device that has been silent for twenty scans reads the same as one
  silent for three.
- Classification still cannot recognise a device that advertises nothing. The
  honest answer for those is still Unknown, and it is still what ArcScan gives.
- A run-together factory label such as `BRW90E2BA` is still shown as a name,
  deliberately: see above.
- `is_gateway` is not recorded per device in the database, so the diagnostic
  report does not state it for a device whose report is built from stored state.
- Loading a very large Inventory is dominated by a v1.8.2 query this release
  does not change; see **Performance** above.
