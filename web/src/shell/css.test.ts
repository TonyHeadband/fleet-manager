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
  it("the_header_controls_sit_lites_item_gap_apart", () => {
    // egui's default item spacing, which Lite's header keeps between its title, filters and switch.
    expect(css).toMatch(/\.view-header \{[^}]*gap: var\(--space-item\)/);
  });
  it("the_live_mark_is_the_shared_bar", () => {
    // The canvas: one status bar size everywhere, the egui item gap after it.
    expect(css).not.toMatch(/\.drawer-footer \.dot \{/);
    expect(css).toMatch(/\.drawer-footer \{[^}]*gap: var\(--space-item\)/);
  });
  it("the_view_sits_on_a_canvas", () => {
    // The page colour around outlined sections: a margin, and a gap between them; no section is filled.
    expect(css).toMatch(/\.content \{[^}]*padding: var\(--layout-canvas-pad\);[^}]*gap: var\(--layout-canvas-gap\);/);
    expect(css).toMatch(/\.body \{[^}]*gap: var\(--layout-canvas-gap\);/);
    expect(css).not.toMatch(/\.view-header \{[^}]*background/);
    expect(css).not.toMatch(/\.footer \{[^}]*background/);
    expect(css).not.toMatch(/\.strip/);
  });
  it("the_top_bar_lines_up_with_the_page_under_it", () => {
    // The title starts where a page's content and its tabs do.
    expect(css).toMatch(/\.view-header \{[^}]*padding: 0 var\(--layout-page-x\)/);
  });
  it("a_views_tab_bar_has_room_above_and_below_its_tabs", () => {
    expect(readFileSync("src/ui/ui.css", "utf8")).toMatch(/\.tabbed > \.tabs \{[^}]*padding: var\(--space-item\) var\(--layout-page-x\)/);
  });
});
