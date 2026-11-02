import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("the scrollbar", () => {
  const css = readFileSync("src/styles.css", "utf8");

  it("the_scrollbar_is_drawn_so_it_stays_visible", () => {
    // WebKitGTK's overlay scrollbars hide until hovered; a styled one is always drawn, in the theme.
    expect(css).toMatch(/::-webkit-scrollbar \{[^}]*width: var\(--layout-scrollbar\)/);
    expect(css).toMatch(/::-webkit-scrollbar-thumb \{[^}]*background: var\(--fill\)/);
  });

  it("its_track_is_a_tick_scale", () => {
    // The board's ticks, stood on end: short ones every step, a full-width one every fifth.
    expect(css).toMatch(
      /::-webkit-scrollbar-track \{[^}]*repeating-linear-gradient\(to bottom, var\(--dashed\) 0 1px, transparent 1px var\(--column-tick-step\)\)/,
    );
    expect(css).toMatch(/::-webkit-scrollbar-track \{[^}]*calc\(5 \* var\(--column-tick-step\)\)/);
    // Large enough to read as a scale: the short ticks reach half across.
    expect(css).toMatch(/::-webkit-scrollbar-track \{[^}]*left \/ calc\(var\(--layout-scrollbar\) \/ 2\) 100% no-repeat/);
  });

  it("the_thumb_is_as_wide_as_the_tick_rule", () => {
    // The ticks and the thumb share one strip: no transparent offset narrows the thumb beside a wider rule.
    expect(css).not.toMatch(/::-webkit-scrollbar-thumb \{[^}]*border-left/);
    expect(readFileSync("src/generated/theme.css", "utf8")).toMatch(/--layout-scrollbar: 8px;/);
  });

  it("its_corners_are_square", () => {
    // FUI keeps rigid angles.
    expect(css).not.toMatch(/::-webkit-scrollbar[^{]*\{[^}]*border-radius/);
  });
});
