import { describe, expect, it } from "vitest";
import {
  FRESHNESS_HINT,
  STALE_AFTER_MISSES,
  detectedUnderOverride,
  effectiveTypeSummary,
  freshnessLabel,
  isNoteworthyFreshness,
  missPhrase,
  resolveType,
  rowType,
  typeSourceLabel,
} from "./effectiveType";
import { DEVICE_TYPE_LABEL } from "./discovery";
import type { InventoryDiscovery, InventoryRow } from "../types";

function discovery(patch: Partial<InventoryDiscovery> = {}): InventoryDiscovery {
  return {
    detected_name: "Living Room",
    device_type: "media_device",
    type_confidence: "medium",
    manufacturer: null,
    model_name: null,
    services: [],
    sources: ["mdns"],
    last_discovered_at: null,
    evidence_freshness: "current",
    ...patch,
  };
}

describe("the effective type", () => {
  it("uses ArcScan's own answer when there is no correction", () => {
    const resolved = resolveType({ detectedType: "printer", detectedConfidence: "high" });
    expect(resolved.effectiveType).toBe("printer");
    expect(resolved.typeSource).toBe("automatic");
    expect(resolved.isUserSet).toBe(false);
  });

  it("lets every shipped type be chosen, and keeps the detected answer underneath", () => {
    for (const id of Object.keys(DEVICE_TYPE_LABEL)) {
      const resolved = resolveType({
        userOverride: id,
        detectedType: "media_device",
        detectedConfidence: "medium",
      });
      expect(resolved.effectiveType).toBe(id);
      expect(resolved.typeSource).toBe("user");
      expect(resolved.isUserSet).toBe(true);
      // Clearing the correction has to be able to reveal this again.
      expect(resolved.detectedType).toBe("media_device");
      expect(resolved.detectedConfidence).toBe("medium");
    }
  });

  it("treats an explicit Unknown as an answer and not as Auto", () => {
    const auto = resolveType({ detectedType: "camera", detectedConfidence: "medium" });
    const chosen = resolveType({
      userOverride: "unknown",
      detectedType: "camera",
      detectedConfidence: "medium",
    });
    expect(auto.effectiveType).toBe("camera");
    expect(auto.isUserSet).toBe(false);
    expect(chosen.effectiveType).toBe("unknown");
    expect(chosen.isUserSet).toBe(true);
  });

  it("reads null, undefined and blank as Auto, because none of them is a decision", () => {
    for (const value of [null, undefined, "", "   "]) {
      const resolved = resolveType({ userOverride: value, detectedType: "printer" });
      expect(resolved.effectiveType).toBe("printer");
      expect(resolved.isUserSet).toBe(false);
    }
  });

  it("falls back to Unknown for a device discovery has never reached", () => {
    const resolved = resolveType({});
    expect(resolved.effectiveType).toBe("unknown");
    expect(resolved.detectedConfidence).toBe("unknown");
    // And a correction on such a device still works.
    const corrected = resolveType({ userOverride: "printer" });
    expect(corrected.effectiveType).toBe("printer");
    expect(corrected.isUserSet).toBe(true);
  });

  it("resolves an Inventory row, including one with no discovery record", () => {
    const base = {
      discovery: discovery(),
      user_device_type: null,
    } as Pick<InventoryRow, "discovery" | "user_device_type">;
    expect(rowType(base).effectiveType).toBe("media_device");
    expect(rowType({ ...base, user_device_type: "television" }).effectiveType).toBe("television");
    expect(rowType({ discovery: null, user_device_type: "printer" }).effectiveType).toBe("printer");
    expect(rowType({ discovery: null, user_device_type: null }).effectiveType).toBe("unknown");
  });
});

describe("how the type reads on screen", () => {
  it("attaches confidence to an automatic answer and never to a corrected one", () => {
    expect(
      effectiveTypeSummary(resolveType({ detectedType: "printer", detectedConfidence: "high" })),
    ).toBe("Printer · High confidence");
    // ArcScan has no business grading how sure the operator is.
    expect(
      effectiveTypeSummary(
        resolveType({
          userOverride: "printer",
          detectedType: "media_device",
          detectedConfidence: "medium",
        }),
      ),
    ).toBe("Printer");
  });

  it("does not say Unknown twice", () => {
    expect(effectiveTypeSummary(resolveType({}))).toBe("Unknown");
  });

  it("says who decided", () => {
    expect(typeSourceLabel("user")).toBe("Set by you");
    expect(typeSourceLabel("automatic")).toBe("Detected automatically");
  });

  it("shows what ArcScan thought underneath a correction, and nothing when it thought nothing", () => {
    expect(
      detectedUnderOverride(
        resolveType({
          userOverride: "television",
          detectedType: "media_device",
          detectedConfidence: "medium",
        }),
      ),
    ).toBe("Media device · Medium confidence");
    // No correction: there is nothing to put underneath.
    expect(
      detectedUnderOverride(resolveType({ detectedType: "printer", detectedConfidence: "high" })),
    ).toBeNull();
    // A correction on a device ArcScan could not type: "ArcScan detected:
    // Unknown" is noise, not information.
    expect(detectedUnderOverride(resolveType({ userOverride: "printer" }))).toBeNull();
  });
});

describe("evidence freshness", () => {
  it("has a word for each state and a hint that explains it in scans", () => {
    expect(freshnessLabel("current")).toBe("Current");
    expect(freshnessLabel("aging")).toBe("Getting old");
    expect(freshnessLabel("stale")).toBe("Stale");
    // An unrecognised value reads as current rather than as blank.
    expect(freshnessLabel("elderly")).toBe("Current");
    expect(freshnessLabel(null)).toBe("Current");
    // The hint must say scans, not days: ArcScan only learns when it runs.
    expect(FRESHNESS_HINT.stale).toContain(`${STALE_AFTER_MISSES} scans`);
    for (const hint of Object.values(FRESHNESS_HINT)) {
      expect(hint.toLowerCase()).not.toContain("day");
    }
  });

  it("marks only the states worth drawing attention to", () => {
    expect(isNoteworthyFreshness("current")).toBe(false);
    expect(isNoteworthyFreshness("aging")).toBe(true);
    expect(isNoteworthyFreshness("stale")).toBe(true);
    expect(isNoteworthyFreshness(null)).toBe(false);
  });

  it("counts misses in scans, singular and plural, and says nothing for none", () => {
    expect(missPhrase(0)).toBeNull();
    expect(missPhrase(null)).toBeNull();
    expect(missPhrase(1)).toBe("last seen 1 discovery scan ago");
    expect(missPhrase(4)).toBe("last seen 4 discovery scans ago");
    // Never a date, and never a duration.
    expect(missPhrase(4)).not.toMatch(/day|week|month/i);
  });
});
