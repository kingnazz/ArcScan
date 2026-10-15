import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IconButton } from "./primitives";

describe("IconButton", () => {
  it("keeps an unavailable action focusable while blocking activation", () => {
    const onClick = vi.fn();
    render(
      <IconButton label="SSH: Port 22 is not open" disabled onClick={onClick}>
        <span aria-hidden>SSH</span>
      </IconButton>,
    );

    const button = screen.getByRole("button", { name: "SSH: Port 22 is not open" });
    expect(button.getAttribute("aria-disabled")).toBe("true");
    expect(button.hasAttribute("disabled")).toBe(false);

    button.focus();
    expect(document.activeElement).toBe(button);

    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });
});
