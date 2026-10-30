import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError, PLAN_CHANGED } from "../api/client";
import type { ArchivePlan } from "../api/generated/ArchivePlan";
import type { ArchiveReport } from "../api/generated/ArchiveReport";
import { FakeEventSource } from "../test/fixtures";
import { tasksHref } from "../shell/routes";
import { deferred, go, history, tasks } from "./harness";

// Ports of the Archive dialog's tests in crates/fleet-gui/tests/tasks.rs.

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
});
