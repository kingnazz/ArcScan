import { describe, expect, it } from "vitest";
import {
  CONFIDENCE_HINT,
  DEVICE_TYPE_LABEL,
  confidenceLabel,
  confidenceTone,
  deviceTypeLabel,
  discoveryHaystack,
  discoveryModeLabel,
  hasNameConflict,
  resolveDisplayName,
  serviceName,
  servicesLabel,
  sourceLabel,
  sourcesLabel,
  typeSummary,
} from "./discovery";
import {
  DISCOVERY_QUALITY_HINT,
  discoveryQualityLabel,
  discoverySummaryLine,
  parseDiscoveryReport,
} from "./discovery";
import type { DeviceDiscovery, InventoryDiscovery } from "../types";

function discovery(patch: Partial<InventoryDiscovery> = {}): InventoryDiscovery {
  return {
    detected_name: "Acme LaserFast 400",
    device_type: "printer",
    type_confidence: "high",
    manufacturer: "Hewlett Packard",
    model_name: "LaserFast 400",
    services: ["_ipp._tcp", "_printer._tcp"],
    sources: ["mdns", "ssdp"],
    last_discovered_at: "2026-08-05T09:00:00Z",
    evidence_freshness: "current",
    ...patch,
  };
}

describe("device type labels", () => {
  it("gives every type a word", () => {
    for (const [id, label] of Object.entries(DEVICE_TYPE_LABEL)) {
      expect(label.length).toBeGreaterThan(0);
      expect(deviceTypeLabel(id)).toBe(label);
    }
  });

  it("shows an unrecognised type rather than hiding it", () => {
    // A value from a newer build must still render as something.
    expect(deviceTypeLabel("toaster")).toBe("toaster");
    expect(deviceTypeLabel(null)).toBe("Unknown");
    expect(deviceTypeLabel(undefined)).toBe("Unknown");
  });
});

describe("confidence", () => {
  it("spells each level out and explains it", () => {
    for (const level of ["high", "medium", "low", "unknown"]) {
      expect(confidenceLabel(level)).toMatch(/confidence|established/i);
      expect(CONFIDENCE_HINT[level].length).toBeGreaterThan(20);
    }
  });

  it("never presents an unknown confidence as a real one", () => {
    expect(confidenceLabel(null)).toBe("Not established");
    expect(confidenceLabel("nonsense")).toBe("Not established");
  });

  it("tones weak claims down so they cannot look authoritative", () => {
    expect(confidenceTone("high")).toBe("online");
    expect(confidenceTone("medium")).toBe("accent");
    expect(confidenceTone("low")).toBe("neutral");
    expect(confidenceTone(null)).toBe("neutral");
  });
});

describe("type summary", () => {
  it("reads as type and confidence together", () => {
    expect(typeSummary("printer", "high")).toBe("Printer · High confidence");
    expect(typeSummary("media_device", "medium")).toBe("Media device · Medium confidence");
  });

  it("says Unknown once, not twice", () => {
    expect(typeSummary("unknown", "unknown")).toBe("Unknown");
    expect(typeSummary(null, null)).toBe("Unknown");
  });
});

describe("sources", () => {
  it("names each protocol the way the interface talks about it", () => {
    expect(sourceLabel("mdns")).toBe("mDNS");
    expect(sourceLabel("ssdp")).toBe("SSDP");
    expect(sourceLabel("reverse_dns")).toBe("Reverse DNS");
    expect(sourceLabel("arp_vendor")).toBe("MAC manufacturer");
  });

  it("joins them and leaves the operator out of the list", () => {
    // "You named it" belongs beside the name, not in a list of protocols.
    expect(sourcesLabel(["mdns", "ssdp"])).toBe("mDNS · SSDP");
    expect(sourcesLabel(["user", "mdns"])).toBe("mDNS");
    expect(sourcesLabel([])).toBe("—");
    expect(sourcesLabel(["user"])).toBe("—");
  });
});

describe("service names", () => {
  it("translates the ones a person would not recognise", () => {
    expect(serviceName("_ipp._tcp")).toBe("IPP printing");
    expect(serviceName("_googlecast._tcp")).toBe("Chromecast");
    expect(serviceName("_hap._tcp")).toBe("HomeKit");
    expect(serviceName("MediaRenderer")).toBe("Media playback");
  });

  it("keeps an unknown service rather than dropping it", () => {
    expect(serviceName("_totally-made-up._tcp")).toBe("_totally-made-up._tcp");
    expect(serviceName("")).toBe("");
  });

  it("caps a long list instead of letting one device dominate", () => {
    const many = ["_ipp._tcp", "_http._tcp", "_ssh._tcp", "_smb._tcp", "_raop._tcp"];
    expect(servicesLabel(many, 3)).toBe("IPP printing, Web, SSH, +2 more");
    expect(servicesLabel([])).toBe("—");
  });
});

describe("discovery mode", () => {
  it("says what a scan managed", () => {
    expect(discoveryModeLabel("full")).toBe("mDNS + SSDP");
    expect(discoveryModeLabel("partial")).toBe("Local discovery incomplete");
    expect(discoveryModeLabel("none")).toBe("No local discovery");
    expect(discoveryModeLabel("something-else")).toBe("No local discovery");
  });
});

describe("search", () => {
  it("reaches the detected name, model, type and services", () => {
    const hay = discoveryHaystack(discovery()).toLowerCase();
    expect(hay).toContain("acme laserfast 400");
    expect(hay).toContain("laserfast 400");
    expect(hay).toContain("printer");
    expect(hay).toContain("hewlett packard");
  });

  it("indexes a service under both its protocol name and its friendly one", () => {
    // The technician searching for `_ipp` and the person searching for
    // "printing" must both find the printer.
    const hay = discoveryHaystack(discovery()).toLowerCase();
    expect(hay).toContain("_ipp._tcp");
    expect(hay).toContain("ipp printing");
  });

  it("is empty rather than undefined for a device discovery never reached", () => {
    expect(discoveryHaystack(null)).toBe("");
    expect(discoveryHaystack(undefined)).toBe("");
  });
});

describe("display name", () => {
  const base = {
    custom_name: null as string | null,
    hostname: "office-printer",
    vendor: "Hewlett Packard",
    current_ip: "192.168.1.31",
  };

  it("uses a name the operator typed above everything", () => {
    expect(
      resolveDisplayName({
        ...base,
        custom_name: "Front Office Printer",
        discovery: discovery(),
      }),
    ).toBe("Front Office Printer");
  });

  it("uses a detected name above the reverse-DNS hostname", () => {
    expect(resolveDisplayName({ ...base, discovery: discovery() })).toBe("Acme LaserFast 400");
  });

  it("falls back exactly as it did before discovery existed", () => {
    expect(resolveDisplayName({ ...base, discovery: null })).toBe("office-printer");
    expect(resolveDisplayName({ ...base, hostname: null, discovery: null })).toBe(
      "Hewlett Packard (192.168.1.31)",
    );
    expect(
      resolveDisplayName({ ...base, hostname: null, vendor: null, discovery: null }),
    ).toBe("192.168.1.31");
  });

  it("treats a blank detected name as no name at all", () => {
    expect(
      resolveDisplayName({ ...base, discovery: discovery({ detected_name: "   " }) }),
    ).toBe("office-printer");
  });
});

describe("name conflicts", () => {
  const record = (patch: Partial<DeviceDiscovery> = {}): DeviceDiscovery => ({
    detected_name: "Living Room",
    name_source: "mdns",
    device_type: "television",
    type_confidence: "high",
    type_evidence: [],
    type_conflicts: [],
    manufacturer: null,
    model_name: null,
    model_number: null,
    serial_number: null,
    mdns_hostname: null,
    ssdp_friendly_name: null,
    services: [],
    sources: ["mdns"],
    alternate_names: [],
    ipv6_addresses: [],
    presentation_url: null,
    first_discovered_at: null,
    last_discovered_at: null,
    evidence: [],
    evidence_freshness: "current",
    raw_type_confidence: "high",
    ...patch,
  });

  it("reports a conflict only when the device advertised another name", () => {
    expect(hasNameConflict(record())).toBe(false);
    expect(hasNameConflict(record({ alternate_names: ["[TV] Living Room"] }))).toBe(true);
    expect(hasNameConflict(null)).toBe(false);
  });
});

describe("discovery quality", () => {
  it("has a word for each state, and reads an unknown one as skipped", () => {
    expect(discoveryQualityLabel("complete")).toBe("Complete");
    expect(discoveryQualityLabel("limited")).toBe("Limited");
    expect(discoveryQualityLabel("skipped")).toBe("Skipped");
    expect(discoveryQualityLabel("interrupted")).toBe("Interrupted");
    expect(discoveryQualityLabel("who knows")).toBe("Skipped");
    expect(discoveryQualityLabel(null)).toBe("Skipped");
  });

  it("never claims a firewall, because ArcScan cannot observe one", () => {
    for (const hint of Object.values(DISCOVERY_QUALITY_HINT)) {
      expect(hint.toLowerCase()).not.toContain("firewall");
    }
  });

  it("shows counts for a complete pass and the observed reason otherwise", () => {
    expect(
      discoverySummaryLine({
        discovery_quality: "complete",
        discovery_summary: JSON.stringify({ mdns_responses: 12, ssdp_responses: 8 }),
      }),
    ).toBe("Complete · 12 mDNS · 8 SSDP");

    // Not the counts: a number beside "Limited" invites a reader to treat it as
    // the whole story, and that is exactly the case where it is not.
    expect(
      discoverySummaryLine({
        discovery_quality: "skipped",
        discovery_quality_reason: "Remote subnet",
        discovery_summary: null,
      }),
    ).toBe("Skipped · Remote subnet");

    expect(
      discoverySummaryLine({
        discovery_quality: "limited",
        discovery_quality_reason: "mDNS socket unavailable",
        discovery_summary: JSON.stringify({ mdns_responses: 3, ssdp_responses: 0 }),
      }),
    ).toBe("Limited · mDNS socket unavailable");

    expect(
      discoverySummaryLine({
        discovery_quality: "interrupted",
        discovery_quality_reason: "Scan stopped",
      }),
    ).toBe("Interrupted · Scan stopped");
  });

  it("survives a summary written by a newer or a broken build", () => {
    expect(parseDiscoveryReport(null)).toBeNull();
    expect(parseDiscoveryReport("")).toBeNull();
    expect(parseDiscoveryReport("not json")).toBeNull();
    expect(parseDiscoveryReport("[1,2,3]")?.mdns_responses).toBe(0);
    const parsed = parseDiscoveryReport(
      JSON.stringify({ mdns_responses: 4, something_new: true }),
    );
    expect(parsed?.mdns_responses).toBe(4);
    expect(parsed?.ssdp_responses).toBe(0);
    // A row with no counts still renders a line rather than throwing.
    expect(discoverySummaryLine({ discovery_quality: "complete", discovery_summary: "junk" })).toBe(
      "Complete",
    );
  });
});
