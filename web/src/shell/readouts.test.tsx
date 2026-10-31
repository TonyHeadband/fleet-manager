import { render, screen, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { TaskList } from "../api/generated/TaskList";
import { TaskReadouts, visibleCells } from "./TaskReadouts";
import { tasksHref } from "./routes";

// The board's task counts as a readout rail at the header's right end; on a narrow header its cells give way, the
// least telling first, never clipped mid-cell.

const list = (crashed: number, orphans: number): TaskList =>
  ({
    tasks: [
      { id: "r1", state: "running", stop_cause: null, footprint: { total: 1_000_000_000 } },
      { id: "s1", state: "stopped", stop_cause: "stopped", footprint: { total: 2_000_000_000 } },
      ...Array.from({ length: crashed }, (_, n) => ({ id: `c${n}`, state: "stopped", stop_cause: "crashed", footprint: null })),
    ],
    orphans: Array.from({ length: orphans }, (_, n) => ({
      id: `o${n}`,
      kind: "task_folder",
      path: "/p",
      worktrees: [],
      bytes: 500_000_000,
    })),
    clones_bytes: null,
  }) as unknown as TaskList;

const keys = () => [...document.querySelectorAll(".readout:not([hidden]) .readout-key")].map((k) => k.textContent);

describe("TaskReadouts", () => {
  it("the_task_counts_are_a_rail", () => {
    render(<TaskReadouts tasks={list(0, 0)} stoppedTone="muted" selected={null} />);
    const rail = screen.getByRole("group", { name: "Tasks" });
    expect(keys()).toEqual(["Tasks", "Run", "Stop", "Disk"]);
    expect(within(rail).getByText("02")).toHaveAttribute("aria-label", "2");
    expect(within(rail).getByText("3.0 GB")).toBeInTheDocument();
    const [run, stop] = within(rail).getAllByRole("link") as [HTMLElement, HTMLElement];
    expect(run).toHaveAttribute("href", tasksHref("running", null));
    expect(run.querySelector('.dot[data-tone="done"]')).not.toBeNull();
    expect(stop.querySelector('.dot[data-tone="muted"]')).not.toBeNull();
  });

  it("crash_and_orphan_cells_only_when_non_zero", () => {
    render(<TaskReadouts tasks={list(1, 2)} stoppedTone="muted" selected={null} />);
    expect(keys()).toEqual(["Tasks", "Run", "Stop", "Crash", "Orphan", "Disk"]);
    const crash = screen.getByRole("link", { name: /Crash/ });
    expect(crash).toHaveAttribute("href", tasksHref("crashed", null));
    expect(crash.querySelector('.dot[data-tone="danger"]')).not.toBeNull();
    expect(screen.getByRole("link", { name: /Orphan/ }).querySelector('.dot[data-tone="caution"]')).not.toBeNull();
  });

  it("nothing_before_the_list_arrives", () => {
    render(<TaskReadouts tasks={null} stoppedTone="muted" selected={null} />);
    expect(keys()).toEqual([]);
  });

  it("cells_give_way_least_telling_first", () => {
    const widths = [60, 50, 50, 50, 70];
    const ranks = [6, 5, 4, 3, 1];
    expect(visibleCells(widths, ranks, 1000)).toEqual(new Set([0, 1, 2, 3, 4]));
    expect(visibleCells(widths, ranks, 280)).toEqual(new Set([0, 1, 2, 3, 4]));
    expect(visibleCells(widths, ranks, 215)).toEqual(new Set([0, 1, 2, 3]));
    expect(visibleCells(widths, ranks, 200)).toEqual(new Set([0, 1, 2]));
    expect(visibleCells(widths, ranks, 0)).toEqual(new Set());
  });

  it("the_rail_sits_at_the_headers_right_end", () => {
    expect(readFileSync("src/shell/shell.css", "utf8")).toMatch(/\.view-header \.readout-rail \{[^}]*margin-left: auto;/);
  });
});
