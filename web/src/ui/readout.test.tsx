import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { pad2, Readout, ReadoutRail } from "./Readout";

// A readout rail: KEY value cells, divided by hairlines, the shared telemetry of the header and the footer.

const css = readFileSync("src/ui/ui.css", "utf8");

describe("readouts", () => {
  it("readout_has_key_bar_value", () => {
    const { container } = render(
      <Readout k="Run" tone="done">
        06
      </Readout>,
    );
    expect(container.querySelector(".readout-key")).toHaveTextContent("Run");
    expect(container.querySelectorAll('.readout > .dot[data-tone="done"]')).toHaveLength(1);
    expect(container.querySelector(".readout-value")).toHaveTextContent("06");
  });

  it("readout_link_is_one_cell", () => {
    render(
      <Readout k="Run" href="#tasks">
        06
      </Readout>,
    );
    const link = screen.getByRole("link");
    expect(link).toHaveClass("readout");
    expect(link).toHaveAttribute("href", "#tasks");
  });

  it("a_rail_is_a_named_group", () => {
    render(
      <ReadoutRail label="Tasks">
        <Readout k="Tasks">08</Readout>
      </ReadoutRail>,
    );
    expect(screen.getByRole("group", { name: "Tasks" })).toHaveClass("readout-rail");
  });

  it("pad2", () => {
    expect(pad2(2)).toBe("02");
    expect(pad2(0)).toBe("00");
    expect(pad2(123)).toBe("123");
  });

  it("the_rail_is_hairline_divided_mono_telemetry", () => {
    expect(css).toMatch(/\.readout-rail > \* \+ \* \{[^}]*border-left: 1px solid var\(--border\);/);
    expect(css).toMatch(/\.readout \{[^}]*padding: 0 var\(--layout-readout-x\);/);
    expect(css).toMatch(/\.readout-key \{[^}]*color: var\(--text-low\);[^}]*letter-spacing: var\(--track-caps\);/);
    expect(css).toMatch(
      /\.readout-value \{[^}]*font-family: var\(--font-mono\);[^}]*color: var\(--text-high\);[^}]*letter-spacing: var\(--track-value\);[^}]*font-variant-numeric: tabular-nums;/,
    );
    expect(css).toMatch(/\.readout\[hidden\] \{\s*display: none;/);
    expect(css).toMatch(/\.outlined \{[^}]*border: 1px solid var\(--border\);[^}]*background: none;/);
  });
});
