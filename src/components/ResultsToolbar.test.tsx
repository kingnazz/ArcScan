import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { EMPTY_FILTER } from "../lib/table";
import { ResultsToolbar } from "./ResultsToolbar";

describe("ResultsToolbar export scope", () => {
  it("shows filtered device export only on the Devices view", () => {
    const props = {
      filter: { ...EMPTY_FILTER, query: "printer" },
      onFilterChange: vi.fn(),
      shown: 1,
      total: 12,
      comparison: null,
      onExport: vi.fn(),
      canExport: true,
      onTabChange: vi.fn(),
      handoff: null,
    };

    const { rerender } = render(<ResultsToolbar {...props} tab="devices" />);
    expect(screen.getByRole("button", { name: "Export" })).toBeTruthy();

    rerender(<ResultsToolbar {...props} tab="comparison" />);
    expect(screen.queryByRole("button", { name: "Export" })).toBeNull();

    rerender(<ResultsToolbar {...props} tab="topology" />);
    expect(screen.queryByRole("button", { name: "Export" })).toBeNull();
  });
});
