import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError, PLAN_CHANGED } from "../api/client";
import type { ArchivePlan } from "../api/generated/ArchivePlan";
import type { ArchiveReport } from "../api/generated/ArchiveReport";
import { layout } from "../generated/layout";
import { FakeEventSource } from "../test/fixtures";
import { tasksHref } from "../shell/routes";
import { deferred, go, history, tasks } from "./harness";

// Ports of the Archive dialog's tests in crates/fleet-gui/tests/tasks.rs.

const css = readFileSync("src/tasks/tasks.css", "utf8");

type Pending<T> = ReturnType<typeof deferred<T>>;

/** The client's plan and archive calls, each answered by hand. */
function calls() {
  const plans: Pending<ArchivePlan>[] = [];
  const archives: Pending<ArchiveReport>[] = [];
  const plan = vi.fn(() => {
    const d = deferred<ArchivePlan>();
    plans.push(d);
    return d.promise;
  });
  const archive = vi.fn(() => {
    const d = deferred<ArchiveReport>();
    archives.push(d);
    return d.promise;
  });
  return { plans, archives, over: { archivePlan: plan, orphanPlan: plan, archive, archiveOrphan: archive } };
}

const oneWorktreePlan = (uncommitted: number): ArchivePlan => ({
  items: [{ path: "/p/bravo/core", bytes: 1_000, uncommitted_files: uncommitted, stays: false }],
  total: 1_000,
});

const dialog = () => within(screen.getByRole("dialog"));
const passportButton = (name: string) => within(screen.getByRole("region", { name: "Passport" })).getByRole("button", { name });
const archiveButton = () => dialog().getByRole("button", { name: "Archive" });

async function open() {
  await userEvent.click(passportButton("Archive…"));
}

async function land<T>(d: Pending<T>, value: T) {
  await act(async () => d.resolve(value));
}

beforeEach(() => {
  FakeEventSource.reset();
  localStorage.clear();
  window.location.hash = "";
});

describe("Archive dialog", () => {
  it("the_archive_dialog_warns_about_uncommitted_files_and_focuses_cancel", async () => {
    const { plans, over } = calls();
    const { client } = await tasks("s1", undefined, over);
    await open();
    expect(dialog().getByRole("heading", { name: "Archive bravo?" })).toBeInTheDocument();
    // The plan has not arrived: nothing to confirm yet.
    expect(dialog().getByText("Measuring…")).toBeInTheDocument();
    expect(archiveButton()).toBeDisabled();
    expect(client.archivePlan).toHaveBeenCalledWith("s1");
    await land(plans[0]!, {
      items: [
        { path: "/p/bravo/core", bytes: 8_000_000_000, uncommitted_files: 3, stays: false },
        { path: "/p/bravo", bytes: 4_000, uncommitted_files: 0, stays: true },
      ],
      total: 8_000_000_000,
    });
    expect(dialog().getByText("What archiving does:")).toBeInTheDocument();
    expect(dialog().getByText("3 uncommitted files will be lost")).toBeInTheDocument();
    // The folder's own contents stay: shown as staying, and not part of what is freed.
    expect(dialog().getByText("stays")).toHaveAttribute("title", "Kept on disk: archiving does not remove it");
    expect(dialog().getByText("Frees 8.0 GB")).toBeInTheDocument();
    expect(dialog().getByText("Branches are kept: only uncommitted work is lost.")).toBeInTheDocument();
    expect(archiveButton()).toBeEnabled();
    expect(dialog().getByRole("button", { name: "Cancel" }), "the safe choice must be the default").toHaveFocus();
  });

  it("paths_read_from_the_task_folder_and_the_list_scrolls_in_the_window", async () => {
    const { plans, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, {
      items: [
        { path: "/home/me/fleet-tasks/bravo/fleet/1-fix/core", bytes: 1_000, uncommitted_files: 0, stays: false },
        { path: "/home/me/.local/share/fleet-manager/agents/a1b2", bytes: 100, uncommitted_files: 0, stays: false },
        { path: "/home/me/fleet-tasks/bravo", bytes: 4_000, uncommitted_files: 0, stays: true },
      ],
      total: 1_100,
    });
    // The task folder is named once; each row under it shows only what follows.
    expect(dialog().getByText("/home/me/fleet-tasks/bravo", { selector: ".archive-root code" })).toBeInTheDocument();
    expect(dialog().getByText("fleet/1-fix/core")).toHaveAttribute("title", "/home/me/fleet-tasks/bravo/fleet/1-fix/core");
    // A path elsewhere keeps its end in view: cut from its start.
    const elsewhere = dialog().getByText("/home/me/.local/share/fleet-manager/agents/a1b2").closest("code")!;
    expect(elsewhere).toHaveAttribute("data-cut", "start");
    const css = readFileSync("src/tasks/tasks.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.dialog\.archive-dialog \{[^}]*max-height: calc\(100% - 2 \* var\(--space-row-x\)\);[^}]*display: flex;/);
    expect(css).toMatch(/\.archive-plan \{[^}]*min-height: 0; overflow-y: auto;/);
  });

  it("a_plan_for_another_task_does_not_enable_archive", async () => {
    const { plans, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await userEvent.click(dialog().getByRole("button", { name: "Cancel" }));
    go(tasksHref("all", "c1"));
    await open();
    expect(dialog().getByRole("heading", { name: "Archive charlie?" })).toBeInTheDocument();
    // bravo's plan answers late, while charlie's dialog waits for its own.
    await land(plans[0]!, { items: [], total: 0 });
    expect(dialog().getByText("Measuring…")).toBeInTheDocument();
    expect(archiveButton()).toBeDisabled();
  });

  it("a_finished_archive_is_announced_with_what_it_freed_or_kept", async () => {
    const { plans, archives, over } = calls();
    const { client, tasks: fetch } = await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, { items: [], total: 0 });
    await userEvent.click(archiveButton());
    expect(client.archive).toHaveBeenCalledWith("s1", { items: [], total: 0 });
    const before = fetch.mock.calls.length;
    await land(archives[0]!, { removed: ["/p/bravo/core"], freed: 8_000_000_000, kept: null });
    expect(screen.queryByRole("dialog"), "the dialog closes").toBeNull();
    expect(fetch.mock.calls.length, "the list is asked for again").toBeGreaterThan(before);
    go(tasksHref("all", "c1"));
    await open();
    await land(plans[1]!, { items: [], total: 0 });
    await userEvent.click(archiveButton());
    await land(archives[1]!, {
      removed: [],
      freed: 0,
      kept: "kept /p/charlie: it contains notes.txt; kept /p/charlie/x: it is a symbolic link",
    });
    const lines = await history();
    expect(lines.some((l) => l.endsWith("bravo archived, 8.0 GB freed"))).toBe(true);
    expect(lines.some((l) => l.endsWith("charlie archived, kept /p/charlie"))).toBe(true);
  });

  it("a_failed_archive_says_so_and_closes", async () => {
    const { plans, archives, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, { items: [], total: 0 });
    await userEvent.click(archiveButton());
    await act(async () => archives[0]!.reject(new FleetError("disk on fire")));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(await screen.findByText("Could not archive bravo: disk on fire")).toBeInTheDocument();
  });

  it("a_plan_that_changed_since_it_was_shown_is_measured_again", async () => {
    const { plans, archives, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, oneWorktreePlan(0));
    await userEvent.click(archiveButton());
    await act(async () => archives[0]!.reject(new FleetError(PLAN_CHANGED)));
    const changed = "This task changed since the plan was made. Review it again.";
    expect(dialog().getByText(changed)).toBeInTheDocument();
    // The stale plan is gone and nothing can be confirmed until the fresh one is on screen.
    expect(dialog().getByText("Measuring…")).toBeInTheDocument();
    expect(archiveButton()).toBeDisabled();
    expect(screen.queryByText(/Could not archive/), "a changed plan is not a failure").toBeNull();
    expect(plans).toHaveLength(2);
    await land(plans[1]!, oneWorktreePlan(2));
    expect(dialog().getByText("2 uncommitted files will be lost")).toBeInTheDocument();
    expect(dialog().getByText(changed)).toBeInTheDocument();
    expect(archiveButton()).toBeEnabled();
  });

  it("a_refused_plan_hands_the_focus_back_to_cancel", async () => {
    const { plans, archives, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, oneWorktreePlan(0));
    await userEvent.click(archiveButton());
    // Archive took the focus and went disabled with it, which dropped the focus.
    await act(async () => archives[0]!.reject(new FleetError(PLAN_CHANGED)));
    expect(dialog().getByRole("button", { name: "Cancel" }), "egui refocuses Cancel when nothing has the focus").toHaveFocus();
  });

  it("the_archive_dialog_is_wider_than_the_shared_one", () => {
    // One token for both apps: a step's worktree path, read from the task folder, needs the room.
    expect(layout.archiveDialog).toBe(880);
    expect(layout.archiveDialog).toBeGreaterThan(layout.dialog);
    expect(css).toMatch(/\.dialog\.archive-dialog \{[^}]*width: min\(var\(--layout-archive-dialog\)/);
  });

  it("cancel_is_disabled_while_the_archive_runs", async () => {
    const { plans, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, oneWorktreePlan(0));
    expect(dialog().getByRole("button", { name: "Cancel" })).toBeEnabled();
    await userEvent.click(archiveButton());
    const cancel = dialog().getByRole("button", { name: "Cancel" });
    expect(cancel).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    expect(dialog().getByText("Archiving…")).toBeInTheDocument();
  });

  it("every_open_of_the_archive_dialog_asks_for_a_fresh_plan", async () => {
    const { plans, over } = calls();
    await tasks("s1", undefined, over);
    expect(plans).toHaveLength(0);
    for (const opened of [1, 2, 3]) {
      await open();
      expect(plans).toHaveLength(opened);
      // Its answer never comes: the next open must not wait for it.
      await userEvent.click(dialog().getByRole("button", { name: "Cancel" }));
    }
  });

  it("a_late_plan_from_an_earlier_open_is_not_shown_as_the_fresh_one", async () => {
    const { plans, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await userEvent.click(dialog().getByRole("button", { name: "Cancel" }));
    await open();
    // The first open's request was slow and answers only now, with what the task held back then.
    await land(plans[0]!, oneWorktreePlan(5));
    expect(dialog().getByText("Measuring…")).toBeInTheDocument();
    expect(archiveButton()).toBeDisabled();
    await land(plans[1]!, oneWorktreePlan(2));
    expect(dialog().getByText("2 uncommitted files will be lost")).toBeInTheDocument();
  });

  it("a_late_plan_from_before_a_changed_plan_is_not_shown_as_the_new_one", async () => {
    const { plans, archives, over } = calls();
    // The first plan request answers twice: once on time, once as a late duplicate.
    await tasks("s1", undefined, over);
    await open();
    const first = plans[0]!;
    await land(first, oneWorktreePlan(0));
    await userEvent.click(archiveButton());
    await act(async () => archives[0]!.reject(new FleetError(PLAN_CHANGED)));
    await land(first, oneWorktreePlan(0));
    expect(dialog().getByText("Measuring…")).toBeInTheDocument();
    expect(plans).toHaveLength(2);
    await land(plans[1]!, oneWorktreePlan(4));
    expect(dialog().getByText("4 uncommitted files will be lost")).toBeInTheDocument();
  });

  it("an_orphans_archive_uses_the_orphan_calls", async () => {
    const { plans, archives, over } = calls();
    const { client } = await tasks("task_folder:/p/echo", undefined, over);
    await open();
    expect(client.orphanPlan).toHaveBeenCalledWith("task_folder:/p/echo");
    await land(plans[0]!, { items: [], total: 0 });
    await userEvent.click(archiveButton());
    expect(client.archiveOrphan).toHaveBeenCalledWith("task_folder:/p/echo", { items: [], total: 0 });
    await land(archives[0]!, { removed: [], freed: 1_000_000, kept: null });
    expect(await screen.findByText("echo archived, 1.0 MB freed")).toBeInTheDocument();
  });

  it("an_archive_landing_after_the_dialog_moved_on_only_tells_the_footer", async () => {
    const { plans, archives, over } = calls();
    await tasks("s1", undefined, over);
    await open();
    await land(plans[0]!, { items: [], total: 0 });
    await userEvent.click(archiveButton());
    // The person leaves the Tasks view while the archive runs.
    go("#kanban");
    expect(screen.queryByRole("dialog")).toBeNull();
    await land(archives[0]!, { removed: [], freed: 8_000_000_000, kept: null });
    expect(await screen.findByText("bravo archived, 8.0 GB freed")).toBeInTheDocument();
    go(tasksHref("all", "s1"));
    await screen.findByRole("list", { name: "Tasks" });
    expect(screen.queryByRole("dialog"), "nothing reopens").toBeNull();
  });

  /** bravo's archive runs while charlie's dialog, with its plan, is open. */
  async function archivingBravoUnderCharlie() {
    const c = calls();
    await tasks("s1", undefined, c.over);
    await open();
    await land(c.plans[0]!, oneWorktreePlan(0));
    await userEvent.click(archiveButton());
    // The address moves on (Back, a link) while the archive runs, and charlie's dialog takes the screen.
    go(tasksHref("all", "c1"));
    await open();
    await land(c.plans[1]!, oneWorktreePlan(1));
    return c;
  }

  const charliesDialogIsUntouched = () => {
    expect(dialog().getByRole("heading", { name: "Archive charlie?" })).toBeInTheDocument();
    expect(dialog().getByText("1 uncommitted file will be lost")).toBeInTheDocument();
    expect(archiveButton()).toBeEnabled();
  };

  it("an_archive_landing_while_another_tasks_dialog_is_open_leaves_that_dialog_open", async () => {
    const { archives } = await archivingBravoUnderCharlie();
    await land(archives[0]!, { removed: [], freed: 8_000_000_000, kept: null });
    expect(await screen.findByText("bravo archived, 8.0 GB freed")).toBeInTheDocument();
    charliesDialogIsUntouched();
  });

  it("a_plan_refused_while_another_tasks_dialog_is_open_is_a_message", async () => {
    const { plans, archives } = await archivingBravoUnderCharlie();
    await act(async () => archives[0]!.reject(new FleetError(PLAN_CHANGED)));
    // Not charlie's plan that changed: nothing is measured again, and bravo's refusal goes to the footer.
    expect(await screen.findByText(`Could not archive bravo: ${PLAN_CHANGED}`)).toBeInTheDocument();
    expect(plans).toHaveLength(2);
    expect(dialog().queryByText(/changed since the plan was made/)).toBeNull();
    charliesDialogIsUntouched();
  });
});
