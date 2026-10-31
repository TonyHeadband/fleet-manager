import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
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

  it("the_entry_css_waits_for_motion_and_steps", () => {
    const css = readFileSync("src/shell/motion.css", "utf8");
    expect(css).toMatch(/@media \(prefers-reduced-motion: no-preference\)/);
    expect(css).toMatch(/clip-path: inset\(0 100% 0 0\)/);
    expect(css).toMatch(/var\(--motion-wipe\) steps\(16\)/);
    expect(css).toMatch(/var\(--motion-step-in\) steps\(3\)/);
    expect(css).toMatch(/var\(--i\) \* var\(--motion-stagger\)/);
    expect(readFileSync("src/styles.css", "utf8")).toContain('@import "./shell/motion.css";');
  });

  it("the_marker_grows_in_steps", () => {
    const css = readFileSync("src/shell/motion.css", "utf8");
    const flat = css.replace(/\s+/g, " ");
    expect(flat).toMatch(
      /\.marker \{ transition: height var\(--motion-marker\) steps\(7\), width var\(--motion-marker\) steps\(2\), background-color var\(--motion-marker\) steps\(2\); \}/,
    );
    expect(css).toMatch(/\.drawer-nav a \{[^}]*transition: color var\(--motion-marker\) steps\(2\);/);
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
});
