import { describe, expect, it } from "vitest";
// Read from disk (vitest runs in web/): vitest turns a stylesheet import, ?raw included, into an empty string.
import { readFileSync } from "node:fs";

const css = readFileSync("src/board/board.css", "utf8");
const rule = (selector: string) => css.match(new RegExp(`(^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} \\{([^}]*)\\}`))?.[2] ?? "";

describe("board.css", () => {
  it("every_card_has_the_same_height", () => {
    expect(rule(".card")).toMatch(
      /grid-template-rows: var\(--card-row-title\) var\(--card-row-state\) var\(--card-row-activity\) var\(--card-row-actions\)/,
    );
    const row = rule(".card-row");
    expect(row).toContain("overflow: hidden");
    expect(row).toContain("white-space: nowrap");
  });
  it("board_css_uses_tokens_only", () => {
    // Every board size comes from --card-*, --column-* or --space-* (Lite's theme.rs); a 1px border is the only literal.
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });
});
