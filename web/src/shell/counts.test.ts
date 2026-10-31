import { describe, expect, it } from "vitest";
import type { Orphan } from "../api/generated/Orphan";
import type { Task } from "../api/generated/Task";
import { agent, task } from "../test/fixtures";
import { fleetCounts, formatBytes, taskCounts } from "./counts";

// Ported from crates/fleet-gui/tests/app.rs `task_list` and `header_counts_show_crashed_and_orphans_only_when_present`.
const list = (more: Task[] = [], orphans: Orphan[] = []) => ({
  tasks: [
    task("r1", "running", null, 1_000_000_000),
    task("s1", "stopped", "stopped", 2_000_000_000),
    // Archived tasks hold nothing and are not counted.
    task("a1", "archived", "stopped", 9_000_000_000),
    ...more,
  ],
  orphans,
  clones_bytes: null,
});
const orphan = { id: "task_folder:/p/lost", kind: "task_folder", path: "/p/lost", worktrees: [], bytes: 500_000_000 } as Orphan;

describe("taskCounts", () => {
  it("counts running and stopped, and what they hold", () => {
    expect(taskCounts(list())).toEqual({ running: 1, stopped: 1, crashed: 0, orphans: 0, held: 3_000_000_000 });
  });
  it("counts a crash as stopped too, and orphans with what they hold", () => {
    expect(taskCounts(list([task("c1", "stopped", "crashed", 0)], [orphan]))).toEqual({ running: 1, stopped: 2, crashed: 1, orphans: 1, held: 3_500_000_000 });
  });
});

describe("formatBytes", () => {
  it("format_bytes_uses_si_units_with_one_decimal", () => {
    for (const [bytes, want] of [
      [0, "0 B"],
      [999, "999 B"],
      [12_000, "12.0 KB"],
      [1_500_000, "1.5 MB"],
      [7_600_000_000, "7.6 GB"],
      [2_340_000_000_000, "2.3 TB"],
    ] as const) {
      expect(formatBytes(bytes), String(bytes)).toBe(want);
    }
  });
});

describe("fleetCounts", () => {
  it("counts_the_agents_and_those_that_need_you", () => {
    expect(fleetCounts([agent({ id: "a", status: "needs_you" }), agent({ id: "b" }), agent({ id: "c" })])).toEqual({ agents: 3, needYou: 1 });
    expect(fleetCounts([])).toEqual({ agents: 0, needYou: 0 });
  });
});
