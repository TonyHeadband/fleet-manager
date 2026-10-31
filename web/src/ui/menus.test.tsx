import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Every selector's menu (FilterMenu, so the board's filters, the drawer's pickers and the views' menus) and the
// footer's History popup slide open fluidly, not at once: no steps, no accent edge.

const css = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
const motion = css.slice(css.indexOf("@media (prefers-reduced-motion: no-preference)"));

describe("menus reveal", () => {
  it("a_menu_slides_down_fluidly", () => {
    expect(motion).toMatch(/\.menu \{ animation: fui-menu var\(--motion-menu\) var\(--ease-push\) both; \}/);
    expect(css).not.toMatch(/\.menu::after|fui-menu-edge/);
    expect(css).toMatch(/@keyframes fui-menu \{ from \{ clip-path: inset\(0 0 100% 0\); \} to \{ clip-path: inset\(0\); \} \}/);
  });

  it("the_history_popup_slides_up_from_the_footer", () => {
    expect(motion).toMatch(/\.history-popup \{ animation: fui-menu-up var\(--motion-menu\) var\(--ease-push\) both; \}/);
    expect(css).toMatch(/@keyframes fui-menu-up \{ from \{ clip-path: inset\(100% 0 0 0\); \} to \{ clip-path: inset\(0\); \} \}/);
  });

  it("the_reveal_has_its_token", () => {
    expect(readFileSync("src/generated/theme.css", "utf8")).toContain("--motion-menu: 200ms;");
  });
});
