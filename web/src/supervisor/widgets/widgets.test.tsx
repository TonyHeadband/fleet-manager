import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StorageSummary } from "../../api/generated/StorageSummary";
import type { Task } from "../../api/generated/Task";
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

const SUMMARY: StorageSummary = {
  worktrees_bytes: 5000,
  build_bytes: 1500,
  repos_bytes: 2000,
  transcripts_bytes: 300,
  daemon_bytes: 400,
  disk_free_bytes: 90_000,
  disk_total_bytes: 500_000,
};
const joinedTables = () => screen.getAllByRole("table", { hidden: true }).map((t) => t.textContent ?? "").join(" ");

/** The daemon, which these widgets do not read. */
const live = { baseUrl: "", client: fakeClient({ storage: vi.fn().mockResolvedValue(SUMMARY) }), onArchive: () => {} };

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

  it("storage_bar_shows_the_routes_live_use_against_the_disks_free_space", async () => {
    const storage = vi.fn().mockResolvedValue(SUMMARY);
    render(STORAGE.render({ persona: "me", agents: [], tasks: [], loaded: true, ...live, client: fakeClient({ storage }) }));
    await waitFor(() => expect(joinedTables()).toMatch(/worktrees\s*5\.0 KB/));
    const joined = joinedTables();
    expect(joined).toMatch(/build output\s*1\.5 KB/);
    expect(joined).toMatch(/repo copies\s*2\.0 KB/);
    expect(joined).toMatch(/kept chats\s*300 B/);
    expect(joined).toMatch(/daemon files\s*400 B/);
    // The scale ends at what Fleet Manager holds (9.2 KB) plus the disk's free space (90 KB).
    expect(screen.getAllByText("99.2 KB").length).toBeGreaterThan(0);
    expect(storage).toHaveBeenCalledTimes(1);
  });

  it("storage_reads_the_route_again_when_the_tasks_change", async () => {
    const storage = vi.fn().mockResolvedValue(SUMMARY);
    const props = { persona: "me", agents: [], loaded: true, ...live, client: fakeClient({ storage }) };
    const { rerender } = render(STORAGE.render({ ...props, tasks: [fullTask("a", "a", "running", null, { persona: "me" })] }));
    await waitFor(() => expect(storage).toHaveBeenCalledTimes(1));
    rerender(STORAGE.render({ ...props, tasks: [fullTask("a", "a", "stopped", "stopped", { persona: "me" })] }));
    await waitFor(() => expect(storage).toHaveBeenCalledTimes(2));
  });

  it("storage_lists_the_biggest_unarchived_merged_or_stopped_tasks_largest_first", async () => {
    const sized = (bytes: number) => ({ footprint: withItems([item("core worktree", bytes)]) });
    const merged = { repos: [{ repo: "core", changed: true }], prs: [{ repo: "core", state: "merged" }] } as unknown as Partial<Task>;
    const tasks = [
      fullTask("small", "small", "stopped", "finished", { persona: "me", ...sized(1_000) }),
      fullTask("big", "big", "stopped", "crashed", { persona: "me", ...sized(9_000_000) }),
      fullTask("merged", "merged", "running", null, { persona: "me", ...merged, ...sized(5_000_000) }),
      fullTask("running", "running", "running", null, { persona: "me", ...sized(8e9) }),
      fullTask("gone", "gone", "archived", "stopped", { persona: "me", ...sized(9e9) }),
      fullTask("other", "other", "stopped", "stopped", { persona: "other", ...sized(9e9) }),
    ];
    render(STORAGE.render({ persona: "me", agents: [], tasks, loaded: true, ...live }));
    const rows = within(screen.getByRole("list", { name: "Biggest tasks to clean up" })).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual([
      "bigCrashed9.0 MBArchive",
      "mergedMerged5.0 MBArchive",
      "smallFinished1.0 KBArchive",
    ]);
  });

  it("storage_archive_button_hands_the_task_to_the_archive_dialog", async () => {
    const onArchive = vi.fn();
    const tasks = [fullTask("t1", "alpha", "stopped", "stopped", { persona: "me" })];
    render(STORAGE.render({ persona: "me", agents: [], tasks, loaded: true, ...live, onArchive }));
    await userEvent.click(screen.getByRole("button", { name: "Archive alpha" }));
    expect(onArchive).toHaveBeenCalledWith({ kind: "task", id: "t1", name: "alpha" });
  });

  it("agents_by_status_has_no_talking_bar", () => {
    render(AGENTS_BY_STATUS.render({ persona: "me", agents, tasks: [], loaded: true, ...live }));
    const joined = joinedTables();
    expect(joined).not.toMatch(/Talking/);
    expect(joined).toMatch(/Queued/);
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
