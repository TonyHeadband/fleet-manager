import { describe, expect, it } from "vitest";
import type { GateRun } from "../api/generated/GateRun";
import type { Task } from "../api/generated/Task";
import type { TaskPr } from "../api/generated/TaskPr";
import { agent, workspace } from "../test/fixtures";
import { workStage } from "./stage";

const URL = "https://github.com/o/r/pull/499";
const pr = (state: TaskPr["state"], url = URL): TaskPr => ({ url, repo: "core", state, merged_at: null, ci: "unknown", failing_checks: [], can_merge: false });
const run = (exit: number, pr_url: string | null = null): GateRun => ({
  at: 1,
  base: "main",
  steps: [
    { name: "fmt", command: "cargo fmt", because: [], exit_code: 0, duration_secs: 1, output_tail: "", counted: false },
    { name: "web", command: "pnpm test", because: [], exit_code: exit, duration_secs: 9, output_tail: "", counted: false },
  ],
  unmatched: [],
  pr_url,
});
const task = (over: Partial<Task> = {}): Task =>
  ({ id: "a1", state: "running", repos: [{ ...workspace("core"), changed: false }], prs: [], gate_runs: [], ...over }) as unknown as Task;
const committed = [{ ...workspace("core", true), changed: true }];

describe("workStage", () => {
  it("nothing_before_any_work", () => {
    expect(workStage(agent(), task())).toBeNull();
    expect(workStage(agent(), undefined)).toBeNull();
  });

  it("changes_then_commits_on_the_branch", () => {
    expect(workStage(agent({ has_changes: true }), task())).toMatchObject({ text: "Changes" });
    expect(workStage(agent({ has_changes: true }), task({ repos: committed }))).toMatchObject({ text: "Committed" });
  });

  it("a_failed_gate_names_its_step", () => {
    expect(workStage(agent({ has_changes: true }), task({ repos: committed, gate_runs: [run(1)] }))).toEqual({
      text: "Gate failed",
      tone: "danger",
      title: "The gate's web step failed",
    });
  });

  it("an_open_pr_shows_its_number_from_the_gate_before_github_is_checked", () => {
    expect(workStage(agent(), task({ repos: committed, gate_runs: [run(0, URL)] }))).toEqual({ text: "PR #499", tone: "active", title: `PR open: ${URL}`, url: URL });
    expect(workStage(agent(), task({ repos: committed, prs: [pr("open")] }))).toMatchObject({ text: "PR #499" });
    // The same PR seen twice counts once; two repos' PRs are counted.
    expect(workStage(agent(), task({ repos: committed, prs: [pr("open")], gate_runs: [run(0, URL)] }))).toMatchObject({ text: "PR #499" });
    expect(workStage(agent(), task({ repos: committed, prs: [pr("open"), pr("open", "https://github.com/o/s/pull/7")] }))).toMatchObject({ text: "2 PRs open" });
  });

  it("merged_wins_and_a_closed_pr_says_so", () => {
    expect(workStage(agent({ has_changes: true }), task({ repos: committed, prs: [pr("merged")], gate_runs: [run(0, URL)] }))).toEqual({
      text: "Merged",
      tone: "done",
      title: "Every PR of the task is merged",
      url: URL,
    });
    // Two PRs, two repos: no single one to link to.
    expect(workStage(agent(), task({ repos: committed, prs: [pr("merged"), pr("merged", "https://github.com/o/s/pull/7")] }))).not.toHaveProperty("url");
    expect(workStage(agent(), task({ repos: committed, prs: [pr("closed")] }))).toMatchObject({ text: "PR closed", tone: "caution" });
    expect(workStage(agent(), task({ done_by_hand: true }))).toMatchObject({ text: "Done by hand", tone: "done" });
  });
});
