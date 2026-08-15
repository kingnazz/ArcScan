# ArcScan 1.8.2

**Better device discovery.**

v1.8.0 gave ArcScan a persistent Inventory and a Changes list. v1.8.1 tidied the
window. v1.8.2 is about a different question: not *what is on this network*,
which ArcScan already answered, but *what are these things*.

The answer comes from the devices themselves. Printers, televisions, routers,
cameras, media players and smart-home equipment already announce what they are
over two protocols built for exactly that purpose. ArcScan now asks, and shows
what it hears — along with where each answer came from and how much of it to
believe.

Install over 1.8.x, 1.7.x or 1.6.x without losing anything. Every scan, device,
name, note, status, network and date is kept, and the database migrates in
place.

---

## What is new

### mDNS discovery

ArcScan asks the local link which services exist on it, then follows up only on
what the link named. That is one enumeration query and a short round of
follow-ups — less traffic than a fixed list of service names, and more complete
than any fixed list can be.

It speaks as a *one-shot querier*: an ephemeral port, the unicast-response bit
set, a few seconds of listening, then the socket closes. ArcScan never binds
port 5353, never joins a group it has to leave, never answers a query, and keeps
nothing between scans.

### SSDP discovery

A standards-compliant `M-SEARCH` for `ssdp:all` and `upnp:rootdevice`, with a
small `MX` so a hundred devices do not all reply in the same millisecond. Where
a device advertises a description document, ArcScan reads it for the
manufacturer and model — subject to the rules below, which are the strictest
part of this release.

### Better detected names

A device that publishes `Acme LaserFast 400` over mDNS is no longer shown as
`192.168.1.31`. The order is fixed and deterministic:

1. a name you typed
2. a high-confidence mDNS name
3. a high-confidence SSDP friendly name
4. the reverse-DNS hostname
5. the manufacturer plus an established device type
6. an mDNS host name
7. the address
8. the MAC address

**A name you typed always wins.** Unconditionally, before any other rule is
consulted. Discovery can name a device that had no name; it can never rename one
you named.

Names that describe a category rather than a device — `printer`, `android`,
`device`, `UPnP Device` — are demoted below the hostname, because two of them on
one network would read as the same thing. `printer` loses to `hp-4th-floor`;
`Front Office Printer` does not.

### Device types, with confidence and evidence

Fourteen types, and four words for how sure ArcScan is.

| Confidence | What it means |
|---|---|
| **High** | The device declared its own kind through a protocol built for the purpose, *and* an independent fact agrees |
| **Medium** | One protocol-level declaration, uncorroborated |
| **Low** | Inferred from an open port, a manufacturer or a name |
| **Unknown** | Nothing supports a type — preferred over a guess |

A word rather than a number, because there is no sense in which a printer
service is 0.7 of a printer, and a percentage invites arithmetic that is not
justified.

The device drawer shows the evidence, not just the verdict:

```
Device type
Printer · High confidence

Evidence
mDNS _ipp._tcp
Hewlett Packard manufacturer
TCP 631 and 9100
```

Where the evidence supports more than one answer, both are shown. A NAS that
also serves media is not a contradiction to hide.

### Discovery sources

Every detected name, model and type records which protocol it came from, and the
Inventory, the drawer and the export all say so. Nothing is presented as a fact
ArcScan simply knows.

---

## Local, read-only, and no credentials

This is the part worth reading carefully.

* **Local only.** Discovery runs only when the scan's target is inside a subnet
  this computer is attached to. Remote-subnet scans, routed targets and public
  targets never send a multicast packet. The multicast TTL is 1, so nothing
  leaves the local link even if a router would have forwarded it.
* **No credentials.** ArcScan sends none, has none, and asks for none.
* **No cloud service.** Nothing about your network, your devices or your
  inventory is sent anywhere. Discovery talks to the local link and to nothing
  else.
* **Read-only.** ArcScan asks questions. It sets nothing, changes nothing, and
  logs into nothing.
* **Bounded.** Every window, every count, every document size has a hard limit.
  A `/24` and a `/16` cost the same discovery pass, because discovery is a
  conversation with the link, not a sweep of addresses.

### Description URLs

An SSDP `LOCATION` is a URL chosen by whatever answered a multicast query.
Anything on the link can answer, and nothing about the response is
authenticated. ArcScan treats it accordingly.

Refused outright: any scheme but plain `http`; embedded credentials; fragments;
IPv6 literals; ports 0, 22, 23, 25, 465 and 587; control characters anywhere;
and anything past a length cap.

Then the destination has to be *inside the local network the scan actually ran
against*. Loopback, link-local (including the cloud-metadata address),
multicast, broadcast and the unspecified address are refused — and so are other
private subnets, which is the point of scoping to the scanned network rather
than to "private addresses". A host name that answers with one local address and
one public one is refused entirely.

Unusual spellings of an address (`0x7f.0.0.1`, `2130706433`, `0177.0.0.1`) are
normalised before the check, so none of them slips past.

The approved address is what the connection uses. There is no second name
resolution between the check and the connect, so a name that validated as local
cannot become public in between. Redirects are refused rather than followed, and
no system proxy is consulted.

**`https` is refused deliberately.** A local device serves its description under
a self-signed certificate for an address, which nothing can verify. Supporting
it would mean shipping a TLS stack with verification switched off — a
meaningless connection with a reassuring padlock. Not fetching is the honest
answer, and the SSDP headers still carry a manufacturer and a device type.

### Description documents

A document containing `<!DOCTYPE` is refused before a single field is read.
That one rule removes external entities, parameter entities and expansion bombs
together, and no UPnP description has a legitimate use for one.

Only the five predefined entities and numeric character references decode, each
into exactly one character, so there is no entity table to poison. Nesting is
capped with an explicit stack rather than recursion. Document size, field
length, service count and element count are all bounded.

Every value comes out as plain text with control characters stripped. Markup
inside a field arrives as characters — `&lt;script&gt;` becomes `<script>` *as
text* — and the interface renders it as text. Icons are counted and never
fetched. A device's own web page is recorded and never opened.

---

## A quiet Changes inbox

Discovery could easily have turned Changes into noise. It does not.

Events are recorded only for changes worth a person's attention: a
high-confidence detected name that changed materially, a high-confidence device
type that changed, a meaningful service that appeared, a service that stopped
being advertised, a manufacturer or model that changed.

Nothing is recorded for:

* the first time ArcScan hears a device — that is a baseline, not a change
* anything below high confidence on either side
* whitespace, casing or punctuation
* TTLs, `CACHE-CONTROL`, `BOOTID`, `CONFIGID`, `SEARCHPORT`, `SERVER` banners
* TXT key ordering, or a device repeating what it already said
* a description that could not be fetched
* a service missing from **fewer than two consecutive** discovery-complete
  scans — one missed multicast response is ordinary on Wi-Fi, and reporting a
  removal on it would be wrong

A stopped scan records no discovery events at all, and a scan that ran discovery
is never compared on discovery-derived facts against one that could not.

---

## Nothing about identity changed

Device identity still resolves by MAC, then hostname-and-vendor, then address,
scoped to a network — exactly as in v1.8.1.

A detected name is **evidence attached to a device**, never a key. It does not
match devices, does not merge them, and does not cross a network scope. Neither
does an mDNS instance name or a UPnP UDN: both are recorded for continuity
within one network and are never compared across two.

A device that gains discovery keeps its id, its identity key, its first-seen
date, its name, its notes and its status.

---

## Elsewhere in the app

* **Inventory** — five optional columns (Type, Detected name, Model, Discovered
  by, Last discovered) and a device-type filter. All off by default; the compact
  default set is the point of that table. Search reaches detected names, models,
  types and services, and indexes services under both their protocol name and
  their friendly one.
* **Device drawer** — a Discovery section with the detected name, type and
  confidence, manufacturer, model, host names, advertised services, the evidence
  behind the type, conflicting readings, alternate names, and any supplemental
  IPv6 addresses.
* **History** — each scan records what its discovery pass managed.
* **Settings** — a Local discovery switch, and under it whether description
  documents may be read. Both on by default.
* **Exports** — eight new Inventory columns. A device no discovery-capable scan
  has reached exports blank cells rather than the word "Unknown", which would
  read as an answer.

**Presence semantics are unchanged.** Present, Missing and Unknown still mean
exactly what they meant in v1.8.0, and an advertisement never decides them. Port
and presence comparison between two scans is exactly as compatible as it was.

---

## Supplemental IPv6

An IPv6 address learned from mDNS is shown as supplemental information, with a
note saying so. **ArcScan scans IPv4 only.** Showing an address is not a claim
that anything was scanned at it.

---

## Not in this release

No scheduled scans, background scanning, notifications, tray mode or launch at
login. No SNMP, no credentials of any kind, no IPv6 scanning, no general UDP
port scanning, no active OS fingerprinting, no packet capture, no vulnerability
or default-password checks, no cloud accounts, no remote agents, no topology
maps. mDNS and SSDP are the only new protocols.

Windows code signing and macOS notarization remain outstanding.

---

## Known limitations

* **One-shot querying misses some responders.** A device that ignores the
  unicast-response bit and answers only to the multicast group is not heard.
  This is the cost of not binding port 5353, which would collide with the
  Bonjour or Avahi responder already running on the machine.
* **A firewall that blocks outbound multicast** means discovery finds nothing.
  ArcScan reports that as no discovery rather than as no devices.
* **`https` description URLs are never read**, for the reason given above.
* **Discovery does not reach routed targets.** That is by design, not a gap.
* **A device that advertises nothing** is exactly as identifiable as it was in
  v1.8.1 — which for many Windows machines and IoT devices is not very.
