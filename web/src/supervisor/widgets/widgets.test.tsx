import { render, screen, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FootprintItem } from "../../api/generated/FootprintItem";
import { agent, fakeClient, fullTask } from "../../test/fixtures";
import { AGENTS_BY_STATUS } from "./AgentsByStatus";
import { STORAGE } from "./Storage";

const item = (label: string, bytes: number): FootprintItem => ({ label, path: `/p/${label}`, bytes, build_bytes: null });
const withItems = (items: FootprintItem[]) => ({
  measured_at: 1,
  items,
  total: items.reduce((s, i) => s + i.bytes, 0),
});

/** The daemon, which these widgets do not read. */
const live = { baseUrl: "", client: fakeClient() };

const agents = [
  agent({ id: "1", persona: "me", status: "working" }),
  agent({ id: "2", persona: "me", status: "working" }),
  agent({ id: "3", persona: "me", status: "needs_you" }),
  agent({ id: "4", persona: "me", status: "stopped" }),
  agent({ id: "5", persona: "me", status: "review", kind: "supervisor" }),
  agent({ id: "6", persona: "other", status: "queued" }),
];

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private cb: (e: { contentRect: { width: number; height: number } }[]) => void) {}
      observe() {
        this.cb([{ contentRect: { width: 400, height: 200 } }]);
      }
      unobserve() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
  delete document.documentElement.dataset.animations;
});

describe("widgets", () => {
  it("agents_by_status_counts_the_personas_live_agents_only", () => {
    render(AGENTS_BY_STATUS.render({ persona: "me", agents, tasks: [], loaded: true, ...live }));
    const group = screen.getByRole("group", { name: "Agents" });
    expect(group).toBeTruthy();
    // The total is the big number: 2 working + 1 needs you.
    expect(within(group).getAllByText("3").length).toBeGreaterThan(0);
    const table = screen.getAllByRole("table", { hidden: true }).map((t) => t.textContent ?? "");
    const joined = table.join(" ");
    expect(joined).toMatch(/Working\s*2/);
    expect(joined).toMatch(/Needs you\s*1/);
    expect(joined).toMatch(/Review\s*0/);
    expect(joined).toMatch(/Queued\s*0/);
  });

  it("agents_by_status_bars_still_grow_when_the_agents_arrive_after_an_empty_first_render", () => {
    const { container, rerender } = render(AGENTS_BY_STATUS.render({ persona: "me", agents: [], tasks: [], loaded: false, ...live }));
    const grow = () => Array.from(container.querySelectorAll(".chart-col-bar")).map((r) => r.getAttribute("data-enter"));
    expect(grow().every((g) => g === null)).toBe(true);
    rerender(AGENTS_BY_STATUS.render({ persona: "me", agents, tasks: [], loaded: true, ...live }));
    expect(grow().every((g) => g === "grow")).toBe(true);
  });

  it("the_total_is_no_data_until_the_snapshot_arrives", () => {
    const stat = (loaded: boolean) => {
      const { container, unmount } = render(AGENTS_BY_STATUS.render({ persona: "me", agents: [], tasks: [], loaded, ...live }));
      const text = container.querySelector(".widget-stat")?.textContent ?? "";
      unmount();
      return text;
    };
    expect(stat(false)).toContain("No data");
    expect(stat(true)).not.toContain("No data");
    expect(stat(true)).toContain("0");
  });

  it("storage_sums_footprints_by_kind", () => {
    const tasks = [
      fullTask("a", "a", "running", null, { persona: "me", footprint: withItems([item("core worktree", 3000), item("build output", 1000)]) }),
      fullTask("b", "b", "stopped", "stopped", { persona: "me", footprint: withItems([item("api worktree", 2000), item("build output", 500)]) }),
      fullTask("c", "c", "archived", "stopped", { persona: "me", footprint: withItems([item("core worktree", 9e9)]) }),
      fullTask("d", "d", "running", null, { persona: "other", footprint: withItems([item("core worktree", 9e9)]) }),
      fullTask("e", "e", "running", null, { persona: "me", footprint: null }),
    ];
    render(STORAGE.render({ persona: "me", agents: [], tasks, loaded: true, ...live }));
    const joined = screen.getAllByRole("table", { hidden: true }).map((t) => t.textContent ?? "").join(" ");
    expect(joined).toMatch(/worktrees\s*5\.0 KB/);
    expect(joined).toMatch(/build output\s*1\.5 KB/);
    expect(joined).not.toMatch(/GB/);
  });

  it("widgets_show_their_title_and_no_footer", () => {
    for (const def of [AGENTS_BY_STATUS, STORAGE]) {
      const { container, unmount } = render(def.render({ persona: "me", agents: [], tasks: [], loaded: true, ...live }));
      const widget = container.querySelector("section.widget") as HTMLElement;
      expect(within(widget).getByRole("heading", { name: def.title })).toBeTruthy();
      // The title alone names a widget: no footer repeats it under the content.
      expect(widget.querySelector("footer")).toBeNull();
      unmount();
    }
    expect(AGENTS_BY_STATUS).toMatchObject({ minW: 2, minH: 2 });
    expect(STORAGE).toMatchObject({ minW: 3, minH: 1 });
  });

  it("widget_motion_is_off_when_motion_is_off", () => {
    const props = { persona: "me", agents, tasks: [], loaded: true, ...live };
    const on = render(AGENTS_BY_STATUS.render(props));
    expect(on.container.querySelector("[data-enter]")).not.toBeNull();
    on.unmount();
    document.documentElement.dataset.animations = "off";
    const off = render(AGENTS_BY_STATUS.render(props));
    expect(off.container.querySelector("svg")).not.toBeNull();
    expect(off.container.querySelector("[data-enter]")).toBeNull();
    expect(readFileSync("src/supervisor/widgets/widgets.css", "utf8")).not.toMatch(/animation|transition|@keyframes/);
  });

  it("widgets_css_uses_tokens_only", () => {
    const css = readFileSync("src/supervisor/widgets/widgets.css", "utf8");
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/border-radius|gradient/);
    expect([...css.matchAll(/(?<![\w-])\d*\.?\d+(px|rem|em)\b/g)].map((m) => m[0]).filter((v) => v !== "1px")).toEqual([]);
  });
});
