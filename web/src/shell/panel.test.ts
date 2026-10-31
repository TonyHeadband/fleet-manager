import { describe, expect, it } from "vitest";
import { layout } from "../generated/layout";
import { panelLayout, sideWidth } from "./panel";

// Beside the board the panel takes the quiet column's place: that column becomes a rail, and the panel is the rest of
// its share, so the other columns keep their width. Too narrow for that, it covers the board.

describe("the side panel", () => {
  it("takes_one_columns_share_less_the_rail_and_the_gap", () => {
    // Five columns over a 1222px section (1px outline each side): 244px each.
    expect(sideWidth(1222, 5)).toBe(244 - layout.rail - layout.canvasGap);
  });

  it("leaves_the_other_columns_their_width", () => {
    const body = 1222;
    const column = (body - 2) / 5;
    // The board, beside the panel and the gap, holds the rail and four whole columns.
    expect(body - layout.canvasGap - sideWidth(body, 5) - 2).toBe(layout.rail + 4 * column);
  });
});

describe("panelLayout", () => {
  it("is_side_while_the_share_is_wide_enough", () => {
    expect(panelLayout(1222, 5, false)).toEqual({ kind: "side" });
  });
  it("is_forced_full_under_the_side_minimum", () => {
    const narrow = 5 * (layout.panelSideMin + layout.rail + layout.canvasGap) + 2 - 5;
    expect(panelLayout(narrow, 5, false)).toEqual({ kind: "full", forced: true });
  });
  it("is_full_on_request", () => {
    expect(panelLayout(1222, 5, true)).toEqual({ kind: "full", forced: false });
  });
});
