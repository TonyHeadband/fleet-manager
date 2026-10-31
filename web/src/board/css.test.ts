import { describe, expect, it } from "vitest";
// Read from disk (vitest runs in web/): vitest turns a stylesheet import, ?raw included, into an empty string.
import { readFileSync } from "node:fs";

const css = readFileSync("src/board/board.css", "utf8");
const rule = (selector: string) => css.match(new RegExp(`(^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} \\{([^}]*)\\}`))?.[2] ?? "";

describe("board.css", () => {
  it("every_card_has_the_same_height", () => {
    expect(rule(".card-in")).toMatch(/grid-template-rows: var\(--card-row-title\) var\(--card-row-actions\) var\(--card-row-state\)/);
    // The avatar's portrait slot, 3:4, beside the title and activity rows; the state row runs the full width under it.
    expect(rule(".card")).toMatch(/--avatar-w: calc\(var\(--avatar-h\) \* 3 \/ 4\)/);
    expect(rule(".avatar")).toMatch(/grid-row: 1 \/ span 2;[^}]*width: var\(--avatar-w\); height: var\(--avatar-h\)/);
    expect(rule(".card")).toMatch(/--avatar-h: calc\(var\(--card-row-title\) \+ var\(--card-row-actions\) \+ var\(--card-row-gap\)\)/);
    const row = rule(".card-row");
    expect(row).toContain("overflow: hidden");
    expect(row).toContain("white-space: nowrap");
    // The web's badge is a pixel taller than the row: the state row lets its frame show whole, top line included.
    expect(rule('.card-row[data-row="state"]')).toContain("overflow: visible");
  });
  it("a_card_is_a_frame_round_its_ground", () => {
    expect(rule(".card")).toMatch(/padding: 1px;[^]*background: var\(--border\);/);
    expect(rule(".card-in")).toContain("background: var(--surface)");
    expect(rule(".card::before")).toMatch(/left: 0; top: 0; bottom: 0; width: var\(--card-edge\)/);
  });
  it("selection_is_an_accent_frame_with_no_fade", () => {
    expect(rule(".card[data-selected]")).toContain("background: var(--fill)");
    expect(rule(".card[data-selected] .card-title")).toContain("color: var(--text-high)");
    expect(css).not.toMatch(/--selection-alpha/);
    expect(css).not.toMatch(/data-corner="nw"|data-corner="sw"/);
  });
  it("live_columns_carry_the_grid", () => {
    const grid = rule(".column:not([data-dim]) .column-body");
    expect(grid.match(/linear-gradient\(/g)).toHaveLength(2);
    expect(grid).toContain("color-mix(in srgb, var(--fill) calc(var(--grid-alpha) * 100%), transparent)");
    expect(grid).toContain("var(--layout-grid) var(--layout-grid)");
    expect(css).not.toMatch(/\.column\[data-dim\] \.column-body \{[^}]*background/);
    expect(rule(".column-header")).not.toMatch(/background/);
  });
  it("board_css_uses_tokens_only", () => {
    // Every board size comes from --card-*, --column-* or --space-* (Lite's theme.rs); a 1px border is the only literal.
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });
});
