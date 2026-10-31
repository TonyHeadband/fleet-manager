import { describe, expect, it } from "vitest";
import { layout } from "../generated/layout";
import { panelLayout } from "./panel";

// Beside the board the panel is a phone's width (Lite's default); the quiet column becomes a rail and the other columns
// share what is left. It covers the board when asked, or when less than a quarter of the board would stay.

describe("panelLayout", () => {
  it("is_side_while_a_quarter_of_the_board_stays_visible", () => {
    expect(panelLayout(1200, false)).toEqual({ kind: "side" });
  });
  it("is_forced_full_when_the_board_would_shrink_under_a_quarter", () => {
    expect(panelLayout(layout.panel / (1 - layout.boardKeep) - 1, false)).toEqual({ kind: "full", forced: true });
  });
  it("is_full_on_request", () => {
    expect(panelLayout(1200, true)).toEqual({ kind: "full", forced: false });
  });
});
