import { describe, expect, it } from "vitest";
import { layout } from "../generated/layout";
import { dragWidth, panelCap, panelLayout, recordWidth, shownWidth } from "./panel";

// Ported from crates/fleet-gui/src/components/preview.rs's panel_layout tests.
describe("panelLayout", () => {
  it("panel_layout_is_side_while_a_quarter_of_the_board_stays_visible", () => {
    expect(panelLayout(1200, 420, false)).toEqual({ kind: "side" });
  });
  it("panel_layout_is_forced_full_when_the_board_would_shrink_under_a_quarter", () => {
    expect(panelLayout(500, 420, false)).toEqual({ kind: "full", forced: true });
  });
  it("panel_layout_is_chosen_full_on_request", () => {
    expect(panelLayout(1200, 420, true)).toEqual({ kind: "full", forced: false });
  });
});

describe("panel widths", () => {
  it("a drag stops at the minimum", () => {
    expect(dragWidth(420, -400, 1200)).toBe(layout.panelMin);
  });
  it("a drag stops short of the width that would force fullscreen", () => {
    expect(dragWidth(420, 2000, 1200)).toBe(panelCap(1200));
    expect(panelLayout(1200, panelCap(1200), false)).toEqual({ kind: "side" });
  });
  it("a shrinking window clamps the shown width to the cap", () => {
    expect(shownWidth(420, 500)).toBe(panelCap(500));
    expect(shownWidth(420, 1200)).toBe(420);
  });
  it("a width at the cap is not recorded: it may be the window's clamp, not the person's choice", () => {
    expect(recordWidth(panelCap(1000), panelCap(1000), 420)).toBe(420);
    expect(recordWidth(500, panelCap(1000), 420)).toBe(500);
  });
});
