import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FakeEventSource, agent, factory, fakeClient, persona, product } from "../test/fixtures";

// The detail panel pushes in from the right as a card is pressed and pulls out as it closes, each with its own
// animation; switching cards moves nothing, and with motion off it opens and closes at once.

const ready = {
  personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }),
  products: vi.fn().mockResolvedValue({ products: [product("p1", "Pie")] }),
};
const start = () => render(<App client={fakeClient(ready)} eventSource={factory} nowSeconds={() => 1300} />);
const snapshot = (...agents: ReturnType<typeof agent>[]) =>
  act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
const panel = () => screen.queryByRole("complementary", { name: "Detail" });
const press = async (goal: string) => userEvent.click(within(screen.getByRole("main")).getByText(goal));
const end = (name: string) => fireEvent.animationEnd(panel()!, { animationName: name });

beforeEach(async () => {
  window.location.hash = "";
  localStorage.clear();
  FakeEventSource.reset();
  start();
  snapshot(agent({ id: "a1", goal: "pie job" }), agent({ id: "a2", goal: "tart job" }));
  await screen.findByText("pie job");
});
afterEach(() => {
  delete document.documentElement.dataset.animations;
});

describe("the panel's push", () => {
  it("the_panel_pushes_in", async () => {
    await press("pie job");
    expect(panel()).toHaveAttribute("data-phase", "opening");
    end("fui-push");
    expect(panel()).toHaveAttribute("data-phase", "open");
  });

  it("closing_waits_for_its_own_animation", async () => {
    await press("pie job");
    end("fui-push");
    await userEvent.click(within(panel()!).getByRole("button", { name: "Close" }));
    expect(panel()).toHaveAttribute("data-phase", "closing");
    end("fui-pull");
    expect(panel()).toBeNull();
  });

  it("closing_then_reselecting_reopens_whole", async () => {
    await press("pie job");
    end("fui-push");
    await press("pie job");
    expect(panel()).toHaveAttribute("data-phase", "closing");
    await press("tart job");
    expect(panel()).toHaveAttribute("data-phase", "opening");
    expect(within(panel()!).getByRole("heading", { name: "tart job" })).toBeInTheDocument();
    // The cancelled close's end, arriving late, unmounts nothing.
    end("fui-pull");
    expect(panel()).not.toBeNull();
  });

  it("switching_cards_does_not_move_the_panel", async () => {
    await press("pie job");
    end("fui-push");
    await press("tart job");
    expect(panel()).toHaveAttribute("data-phase", "open");
  });

  it("motion_off_opens_and_closes_at_once", async () => {
    document.documentElement.dataset.animations = "off";
    await press("pie job");
    expect(panel()).toHaveAttribute("data-phase", "open");
    await press("pie job");
    expect(panel()).toBeNull();
  });

  it("the_push_css", () => {
    const css = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.detail\[data-phase="opening"\] \{[^}]*animation: fui-push var\(--motion-push\) var\(--ease-push\)/);
    expect(css).toMatch(/\.detail\[data-phase="closing"\] \{[^}]*animation: fui-pull var\(--motion-pull\) var\(--ease-pull\)/);
    expect(css).toMatch(/--ease-push: cubic-bezier\(0\.2, 0\.7, 0\.2, 1\);/);
    expect(css).toMatch(/--ease-pull: cubic-bezier\(0\.4, 0, 0\.8, 0\.4\);/);
    expect(css).toMatch(
      /\.detail\[data-phase="opening"\], \.detail\[data-phase="closing"\] \{[^}]*border-left-color: var\(--fill\);[^}]*overflow: hidden;/,
    );
    expect(css).toMatch(
      /\.detail\[data-phase="opening"\] > \*, \.detail\[data-phase="closing"\] > \* \{[^}]*min-width: var\(--panel-final\);/,
    );
    expect(css).toMatch(/@keyframes fui-push \{ from \{ width: 0; \} \}/);
    expect(css).toMatch(/@keyframes fui-pull \{ to \{ width: 0; \} \}/);
  });
});
