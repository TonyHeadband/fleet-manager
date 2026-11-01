import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { entryEnd } from "./entry";
import { FakeEventSource, agent, factory, fakeClient, persona, product } from "../test/fixtures";

// Entering a view: its sections wipe in, then its cards step in; the state clears on the last animation's end, never
// by a timer, and never starts while motion is off.

const ready = {
  personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }),
  products: vi.fn().mockResolvedValue({ products: [product("p1", "Pie")] }),
};
const start = () => render(<App client={fakeClient(ready)} eventSource={factory} nowSeconds={() => 1300} />);
const snapshot = (...agents: ReturnType<typeof agent>[]) =>
  act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
const content = () => document.querySelector(".content")!;
const entering = () => content().hasAttribute("data-entering");
const go = (hash: string) => {
  window.location.hash = hash;
  act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
};
/** Every animation the entry started ends, in the order they would. */
const finish = () => {
  for (const el of content().querySelectorAll(".outlined")) fireEvent.animationEnd(el, { animationName: "fui-wipe" });
  for (const el of content().querySelectorAll(".card, .fleet-row")) fireEvent.animationEnd(el, { animationName: "fui-step-in" });
};

beforeEach(() => {
  window.location.hash = "";
  localStorage.clear();
  FakeEventSource.reset();
});
afterEach(() => {
  delete document.documentElement.dataset.animations;
  vi.unstubAllGlobals();
});

describe("view entry", () => {
  it("entry_runs_once_the_board_is_ready", async () => {
    start();
    await screen.findByRole("banner");
    expect(entering()).toBe(false);
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    expect(entering()).toBe(true);
    finish();
    expect(entering()).toBe(false);
  });

  it("motion_off_never_enters", async () => {
    document.documentElement.dataset.animations = "off";
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    expect(entering()).toBe(false);
    go("#tasks");
    expect(entering()).toBe(false);
  });

  it("reduced_motion_never_enters", async () => {
    vi.stubGlobal("matchMedia", (q: string) => ({
      matches: q.includes("reduce"),
      media: q,
      addEventListener() {},
      removeEventListener() {},
    }));
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    expect(entering()).toBe(false);
  });

  it("every_view_enters_and_the_current_one_does_not_replay", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    finish();
    for (const hash of ["#tasks", "#releases", "#forms", "#kanban"]) {
      go(hash);
      expect(entering(), hash).toBe(true);
      finish();
    }
    await userEvent.click(within(screen.getByRole("navigation", { name: "Drawer" })).getByRole("link", { name: "Kanban" }));
    expect(entering()).toBe(false);
  });

  it("a_route_change_mid_entry_restarts", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    const old = content().querySelector(".footer")!;
    go("#tasks");
    expect(entering()).toBe(true);
    // The old view's sections are gone: a late end from them reaches nothing.
    fireEvent.animationEnd(old, { animationName: "fui-wipe" });
    expect(entering()).toBe(true);
  });

  it("the_entry_ends_on_its_last_animation", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    // One card ends before the footer's wipe: the footer's end is the entry's.
    fireEvent.animationEnd(content().querySelector(".card")!, { animationName: "fui-step-in" });
    fireEvent.animationEnd(content().querySelector(".view-header")!, { animationName: "fui-wipe" });
    expect(entering()).toBe(true);
    fireEvent.animationEnd(content().querySelector(".footer")!, { animationName: "fui-wipe" });
    expect(entering()).toBe(false);
  });

  it("a_full_board_ends_on_its_last_staggered_card", async () => {
    start();
    snapshot(...Array.from({ length: 8 }, (_, n) => agent({ id: `a${n}`, goal: `job ${n}` })));
    await screen.findByText("job 7");
    fireEvent.animationEnd(content().querySelector(".footer")!, { animationName: "fui-wipe" });
    expect(entering()).toBe(true);
    const cards = content().querySelectorAll(".card");
    fireEvent.animationEnd(cards[cards.length - 1]!, { animationName: "fui-step-in" });
    expect(entering()).toBe(false);
  });

  it("a_panel_opened_mid_entry_has_no_wipe", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(await screen.findByText("pie job"));
    expect(entering()).toBe(true);
    expect(screen.getByRole("complementary", { name: "Detail" })).not.toHaveAttribute("data-enters");
  });

  it("a_panel_open_at_the_entry_wipes_with_it", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(await screen.findByText("pie job"));
    finish();
    go("#tasks");
    finish();
    go("#kanban");
    expect(screen.getByRole("complementary", { name: "Detail" })).toHaveAttribute("data-enters");
  });

  it("the_stagger_caps_at_16_in_board_order", async () => {
    start();
    const agents = Array.from({ length: 20 }, (_, n) => agent({ id: `a${n}`, goal: `job ${n}`, status: n < 3 ? "queued" : "working" }));
    snapshot(...agents);
    await screen.findByText("job 19");
    const is = [...content().querySelectorAll<HTMLElement>(".card")].map((c) => c.style.getPropertyValue("--i"));
    expect(is).toEqual([...Array.from({ length: 16 }, (_, n) => String(n)), "15", "15", "15", "15"]);
    // Board order: the Queued column's three cards come first.
    expect(
      within(screen.getByRole("region", { name: "Queued" }))
        .getAllByRole("article")
        .map((c) => c.style.getPropertyValue("--i")),
    ).toEqual(["0", "1", "2"]);
  });

  it("the_entry_css_waits_for_motion_and_runs_smooth", () => {
    const css = readFileSync("src/shell/motion.css", "utf8");
    const flat = css.replace(/\s+/g, " ");
    expect(css).toMatch(/@media \(prefers-reduced-motion: no-preference\)/);
    expect(css).toMatch(/clip-path: inset\(0 100% 0 0\)/);
    expect(flat).toMatch(/animation: fui-wipe var\(--motion-wipe\) var\(--ease-push\) calc\(var\(--after\) \+ var\(--at\)\) both;/);
    expect(flat).toMatch(/animation: fui-step-in var\(--motion-step-in\) ease-out calc\(/);
    expect(css).toMatch(/var\(--i\) \* var\(--motion-stagger\)/);
    // All motion is fluid; switching views, a 1px accent edge leads each section's wipe, moving with it.
    expect(css).not.toMatch(/steps\(/);
    expect(flat).toMatch(
      /\.content\[data-entering\] :is\(\.view-header, \.view-body, \.detail\[data-enters\], \.footer\)::after \{[^}]*width: 1px;[^}]*margin-right: 1px;[^}]*background: var\(--fill\);[^}]*animation: fui-edge var\(--motion-wipe\) var\(--ease-push\) calc\(var\(--after\) \+ var\(--at\)\) both;/,
    );
    expect(readFileSync("src/styles.css", "utf8")).toContain('@import "./shell/motion.css";');
  });

  it("the_marker_grows_smoothly", () => {
    const flat = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    expect(flat).toMatch(
      /\.marker \{ transition: height var\(--motion-marker\) var\(--ease-push\), width var\(--motion-marker\) var\(--ease-push\), background-color var\(--motion-marker\) var\(--ease-push\); \}/,
    );
    expect(flat).toMatch(/\.drawer-nav a \{ transition: color var\(--motion-marker\) var\(--ease-push\); \}/);
  });

  it("a_view_change_enters_after_the_marker", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await screen.findByText("pie job");
    // The first load has no marker to wait for.
    expect(content()).not.toHaveAttribute("data-after-marker");
    finish();
    go("#tasks");
    expect(content()).toHaveAttribute("data-after-marker");
    const css = readFileSync("src/shell/motion.css", "utf8");
    expect(css).toMatch(/\.content\[data-after-marker\] \{[^}]*--after: var\(--motion-marker\);/);
    expect(css).toMatch(/calc\(var\(--after\) \+ var\(--at\)\)/);
  });

  it("the_entry_ends_on_an_animation_that_runs", () => {
    // No cards: the footer's wipe; few: still the footer (it ends later); many: the last staggered card, capped at 16.
    expect(entryEnd(0)).toEqual({ last: "footer", at: 860 });
    expect(entryEnd(1)).toEqual({ last: "footer", at: 860 });
    expect(entryEnd(16)).toEqual({ last: "card", at: 1315 });
    expect(entryEnd(40)).toEqual(entryEnd(16));
  });

  it("an_empty_board_ends_on_the_footer", async () => {
    start();
    snapshot();
    await screen.findByText("Nothing to review");
    expect(entering()).toBe(true);
    fireEvent.animationEnd(content().querySelector(".footer")!, { animationName: "fui-wipe" });
    expect(entering()).toBe(false);
  });

  it("the_panel_keeps_its_width_and_fullscreen_across_views", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(await screen.findByText("pie job"));
    const detail = () => screen.getByRole("complementary", { name: "Detail" });
    const width = detail().style.width;
    await userEvent.click(within(detail()).getByRole("button", { name: "Fullscreen" }));
    go("#tasks");
    go("#kanban");
    expect(detail()).toHaveAttribute("data-layout", "full");
    await userEvent.click(within(detail()).getByRole("button", { name: "Back to the board" }));
    fireEvent.animationEnd(detail(), { animationName: "fui-narrow" });
    expect(detail().style.width).toBe(width);
  });

  it("a_fullscreen_panel_ends_the_entry_on_the_footer", async () => {
    start();
    snapshot(...Array.from({ length: 8 }, (_, n) => agent({ id: `a${n}`, goal: `job ${n}` })));
    await userEvent.click(await screen.findByText("job 0"));
    await userEvent.click(within(screen.getByRole("complementary", { name: "Detail" })).getByRole("button", { name: "Fullscreen" }));
    finish();
    go("#tasks");
    finish();
    go("#kanban");
    expect(entering()).toBe(true);
    // The board is hidden under the panel, so no card animates: the footer's wipe is the last.
    fireEvent.animationEnd(content().querySelector(".footer")!, { animationName: "fui-wipe" });
    expect(entering()).toBe(false);
  });

  it("a_lost_last_animation_still_ends_the_entry", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      start();
      snapshot(...Array.from({ length: 8 }, (_, n) => agent({ id: `a${n}`, goal: `job ${n}` })));
      await screen.findByText("job 7");
      expect(entering()).toBe(true);
      // The last card leaves mid-entry: its end never comes, and the fallback clears the entry once it is due.
      act(() => vi.advanceTimersByTime(entryEnd(8).at + 360 + 1));
      expect(entering()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
