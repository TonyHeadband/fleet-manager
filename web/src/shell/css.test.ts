import { describe, expect, it } from "vitest";
// Read from disk (vitest runs in web/): vitest turns a stylesheet import, ?raw included, into an empty string.
import { readFileSync } from "node:fs";

const css = readFileSync("src/shell/shell.css", "utf8");

describe("shell.css", () => {
  it("is read", () => {
    expect(css).toContain(".shell {");
  });
  it("shell_css_uses_tokens_only", () => {
    // Every shell size comes from --layout-* or --space-* (Lite's theme.rs); a 1px border is the only literal.
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });
  it("the_page_never_scrolls_sideways", () => {
    expect(css).toMatch(/\.shell \{[^}]*overflow: hidden/);
  });
  it("the_header_filter_scrolls_in_its_row_instead_of_wrapping_out_of_sight", () => {
    expect(css).toMatch(/\.view-header \.filter \{[^}]*flex-wrap: nowrap[^}]*overflow-x: auto/);
  });
});
