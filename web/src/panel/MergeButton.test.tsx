import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CiState } from "../api/generated/CiState";
import type { MergedPr } from "../api/generated/MergedPr";
import type { Task } from "../api/generated/Task";
import type { TaskPr } from "../api/generated/TaskPr";
import { FleetError } from "../api/client";
import { fullTask } from "../test/fixtures";
import { useMerge, type MergeApi } from "./MergeButton";

const URL = "https://github.com/o/core/pull/504";
const pr = (over: Partial<TaskPr> = {}): TaskPr => ({ url: URL, repo: "core", state: "open", merged_at: null, ci: "green", failing_checks: [], can_merge: true, merge_method: "rebase", ...over });
const taskWith = (...prs: TaskPr[]): Task => fullTask("t1", "idea", "running", null, { prs });
const merged: MergedPr = { url: URL, repo: "core", number: 504, method: "rebase" };

function Harness({ task, api, onChanged }: { task: Task; api: MergeApi; onChanged: () => void }) {
  const { button, outcome } = useMerge(task, api, onChanged);
  return (
    <div>
      {button}
      {outcome}
    </div>
  );
}

function setup(task: Task, over: Partial<MergeApi> = {}) {
  const api: MergeApi = { refresh: vi.fn().mockResolvedValue(task), merge: vi.fn().mockResolvedValue(merged), ...over };
  const onChanged = vi.fn();
  const view = render(<Harness task={task} api={api} onChanged={onChanged} />);
  return { api, onChanged, ...view };
}

describe("the Merge button", () => {
  it("is_green_and_enabled_when_ci_is_green", () => {
    setup(taskWith(pr()));
    const b = screen.getByRole("button", { name: "Merge" });
    expect(b).toBeEnabled();
    expect(b).toHaveClass("badge");
    expect(b).toHaveAttribute("data-tone", "done");
  });

  it("is_red_and_disabled_when_ci_failed_listing_the_failing_checks_on_hover", () => {
    setup(taskWith(pr({ ci: "red", failing_checks: ["windows", "lint"] })));
    const b = screen.getByRole("button", { name: "Merge" });
    expect(b).toBeDisabled();
    expect(b).toHaveAttribute("data-tone", "danger");
    expect(b).toHaveAttribute("title", expect.stringContaining("windows"));
    expect(b).toHaveAttribute("title", expect.stringContaining("lint"));
  });

  it.each<CiState>(["pending", "unknown"])("is_grey_and_disabled_while_ci_is_%s", (ci) => {
    setup(taskWith(pr({ ci })));
    const b = screen.getByRole("button", { name: "Merge" });
    expect(b).toBeDisabled();
    expect(b).toHaveAttribute("data-tone", "muted");
  });

  it("is_disabled_when_github_says_the_pr_cannot_merge", () => {
    setup(taskWith(pr({ can_merge: false, merge_method: null })));
    expect(screen.getByRole("button", { name: "Merge" })).toBeDisabled();
  });

  it("is_absent_without_an_open_pr", () => {
    setup(taskWith(pr({ state: "merged" })));
    expect(screen.queryByRole("button", { name: "Merge" })).toBeNull();
  });

  it("refreshes_the_prs_once_when_opened_and_tells_the_app", async () => {
    const { api, onChanged } = setup(taskWith(pr({ ci: "pending" })));
    await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
    expect(api.refresh).toHaveBeenCalledTimes(1);
    expect(api.refresh).toHaveBeenCalledWith("t1");
  });

  it("does_not_refresh_a_task_with_no_open_pr", () => {
    const { api } = setup(taskWith());
    expect(api.refresh).not.toHaveBeenCalled();
  });

  it("asks_before_merging_then_merges_and_states_the_outcome", async () => {
    const { api, onChanged } = setup(taskWith(pr()));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Merge" }));
    expect(api.merge).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveTextContent("Merge PR #504 into main with rebase?");
    await userEvent.click(screen.getByRole("button", { name: "Merge PR" }));
    expect(api.merge).toHaveBeenCalledWith("t1", "core");
    expect(await screen.findByRole("status")).toHaveTextContent("Merged PR #504 with rebase.");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onChanged).toHaveBeenCalledTimes(2);
  });

  it("cancel_makes_no_merge_call", async () => {
    const { api } = setup(taskWith(pr()));
    await userEvent.click(screen.getByRole("button", { name: "Merge" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.merge).not.toHaveBeenCalled();
  });

  it("shows_the_daemons_refusal", async () => {
    const refusal = "PR #504 was not merged: base branch policy prohibits the merge";
    const { api } = setup(taskWith(pr()), { merge: vi.fn().mockRejectedValue(new FleetError(refusal)) });
    await userEvent.click(screen.getByRole("button", { name: "Merge" }));
    await userEvent.click(screen.getByRole("button", { name: "Merge PR" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(refusal);
    expect(api.merge).toHaveBeenCalledTimes(1);
  });
});
