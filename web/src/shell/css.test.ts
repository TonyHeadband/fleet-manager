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
  it("the_live_dot_is_lites_size", () => {
    // drawer.rs: an 8px dot, the egui item gap after it.
    expect(css).toMatch(/\.drawer-footer \.dot \{[^}]*var\(--size-live-dot\)/);
    expect(css).toMatch(/\.drawer-footer \{[^}]*gap: var\(--space-item\)/);
  });
  it("the_strip_starts_at_the_left_and_grows_rightwards", () => {
    expect(css).toMatch(/\.strip \{[^}]*justify-content: flex-start/);
  });
  it("the_top_bar_and_footer_are_the_drawers_colour", () => {
    // Their rows run on from the drawer's header and footer, so they take its background.
    expect(css).toMatch(/\.view-header \{[^}]*background: var\(--sunken\)/);
    expect(css).toMatch(/\.footer \{[^}]*background: var\(--sunken\)/);
  });
  it("the_top_bar_lines_up_with_the_page_under_it", () => {
    // The title and the strip start where a page's content and its tabs do.
    expect(css).toMatch(/\.view-header \{[^}]*padding: 0 var\(--layout-page-x\)/);
    expect(css).toMatch(/\.strip \{[^}]*padding: 0 var\(--layout-page-x\)/);
  });
  it("a_views_tab_bar_has_room_above_and_below_its_tabs", () => {
    expect(readFileSync("src/ui/ui.css", "utf8")).toMatch(/\.tabbed > \.tabs \{[^}]*padding: var\(--space-item\) var\(--layout-page-x\)/);
  });
});
