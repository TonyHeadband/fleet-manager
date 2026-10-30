import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../api/generated/Task";
import type { TaskList } from "../api/generated/TaskList";
import { FakeEventSource, NOW, agent, fullTask, product, repoInfo, tasksList } from "../test/fixtures";
import { tasksHref } from "../shell/routes";
import { go, productP, products, snapshot, tasks, totals, withProducts } from "./harness";

// Ports of the passport's tests in crates/fleet-gui/tests/tasks.rs.

const passport = () => within(screen.getByRole("region", { name: "Passport" }));
const button = (name: string) => passport().getByRole("button", { name });

/** egui's `app()` puts r1 on the board, working. */
const onBoard = () => snapshot(agent({ id: "r1", goal: "alpha goal" }));

beforeEach(() => {
  FakeEventSource.reset();
  localStorage.clear();
  window.location.hash = "";
});

describe("Passport", () => {
  it("the_passport_shows_the_goal_repos_footprint_and_log", async () => {
    await tasks("s1");
    // The goal's first line only.
    expect(passport().getByText("bravo goal")).toBeInTheDocument();
    expect(passport().getByText("fleet/core-x")).toBeInTheDocument();
    expect(passport().getByText("from main at abcdef1")).toBeInTheDocument();
    expect(passport().getByText("of which target/")).toBeInTheDocument();
    expect(passport().getByText("7.6 GB")).toBeInTheDocument();
    expect(passport().getByText("measured 5m ago")).toBeInTheDocument();
    expect(passport().getByText("started with core")).toBeInTheDocument();
    expect(passport().getByText("not saved")).toBeInTheDocument();
    expect(passport().getByText("/p/bravo")).toBeInTheDocument();
    expect(passport().getByText("started 2h 0m ago")).toBeInTheDocument();
  });

  it("a_backfilled_task_shows_its_times_as_unknown", async () => {
    const old = fullTask("s1", "bravo", "stopped", "exited", {
      created_at: null,
      times: { queued: 0, working: 0, needs_you: 0, review: 0 },
    });
    await tasks("s1", { tasks: [old], orphans: [], clones_bytes: null });
    expect(passport().getByText("started: unknown")).toBeInTheDocument();
    expect(passport().queryByText("0s"), "unknown is not zero").toBeNull();
    expect(passport().getAllByText("unknown")).toHaveLength(2);
  });

  it("tokens_and_cost_say_not_tracked_yet", async () => {
    await tasks("r1");
    // Once in the totals and once in the passport.
    expect(within(totals()).getByText("not tracked yet")).toBeInTheDocument();
    expect(passport().getByText("not tracked yet")).toBeInTheDocument();
  });

  it("stop_in_the_passport_asks_for_a_second_click", async () => {
    const { client } = await tasks("r1");
    onBoard();
    await userEvent.click(await passport().findByRole("button", { name: "Stop" }));
    expect(client.run).not.toHaveBeenCalled();
    await userEvent.click(button("Confirm Stop"));
    expect(client.run).toHaveBeenCalledWith(expect.objectContaining({ id: "stop" }), expect.objectContaining({ id: "r1" }), undefined);
    expect(passport().queryByRole("button", { name: "Confirm Stop" })).toBeNull();
  });

  it("archive_is_disabled_until_the_task_is_stopped", async () => {
    await tasks("r1");
    onBoard();
    expect(button("Archive…")).toBeDisabled();
    expect(button("Archive…")).toHaveAttribute("title", "stop it first");
    expect(await passport().findByRole("button", { name: "Stop" })).toBeInTheDocument();
    go(tasksHref("all", "s1"));
    expect(button("Archive…")).toBeEnabled();
    expect(passport().queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("archived_tasks_are_read_only", async () => {
    await tasks("a1");
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "Show archived" }));
    go(tasksHref("all", "a1"));
    expect(passport().getByText("delta")).toBeInTheDocument();
    // Archived: read-only, no way back to a board it left.
    expect(passport().queryByRole("button", { name: "Open on board" })).toBeNull();
    expect(passport().queryByRole("button", { name: "Archive…" })).toBeNull();
    expect(passport().queryByRole("button", { name: "Refresh" })).toBeNull();
    expect(passport().getByText(/· archived 1m ago/)).toBeInTheDocument();
  });

  it("open_on_board_shows_the_card_in_the_side_panel", async () => {
    await tasks("r1");
    onBoard();
    await userEvent.click(button("Open on board"));
    expect(await within(screen.getByRole("banner")).findByRole("heading", { name: "Kanban" })).toBeInTheDocument();
    expect(within(screen.getByRole("complementary", { name: "Detail" })).getByText("alpha goal")).toBeInTheDocument();
  });

  it("a_stopped_tasks_diff_shows_in_its_passport", async () => {
    const diff = vi.fn().mockResolvedValue({ workspaces: [{ repo: "core", diff: "+added line", truncated: false }] });
    await tasks("s1", undefined, { diff });
    expect(await passport().findByText("+added line")).toBeInTheDocument();
    expect(diff).toHaveBeenCalledWith("s1");
    await userEvent.click(passport().getByRole("button", { name: "Reload the diff" }));
    expect(diff).toHaveBeenCalledTimes(2);
  });

  it("a_diff_for_a_task_no_longer_selected_never_shows", async () => {
    let answer: (v: unknown) => void = () => {};
    const diff = vi
      .fn()
      .mockImplementationOnce(() => new Promise((r) => (answer = r)))
      .mockResolvedValue({ workspaces: [{ repo: "core", diff: "+charlie line", truncated: false }] });
    await tasks("s1", undefined, { diff });
    go(tasksHref("all", "c1"));
    expect(await passport().findByText("+charlie line")).toBeInTheDocument();
    await act(async () => answer({ workspaces: [{ repo: "core", diff: "+bravo line", truncated: false }] }));
    expect(passport().queryByText("+bravo line")).toBeNull();
  });

  it("refresh_waits_for_a_fetch_in_flight", async () => {
    let land: (l: TaskList) => void = () => {};
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(tasksList())
      .mockImplementationOnce(() => new Promise<TaskList>((r) => (land = r)))
      .mockResolvedValue(tasksList());
    const { client } = await tasks("s1", undefined, { tasks: fetch });
    // The board's stream asks for the list again; that fetch is still out when Refresh is clicked.
    onBoard();
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
    await userEvent.click(passport().getByRole("button", { name: "Refresh" }));
    expect(client.refreshTasks, "a refresh overlapped the fetch in flight").not.toHaveBeenCalled();
    // Once that list lands, the queued refresh goes, so its newer list is the last one applied.
    await act(async () => land(tasksList()));
    await waitFor(() => expect(client.refreshTasks).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(3));
  });

  it("mark_done_only_for_path_only_tasks", async () => {
    const withUrl = fullTask("u1", "uniform", "stopped", "stopped", {
      repos: [{ ...fullTask("x", "x", "stopped", null).repos[0]!, repo: "web" }],
    });
    const done = fullTask("d1", "delta2", "stopped", "stopped", { done_by_hand: true });
    const list = {
      tasks: [fullTask("s1", "bravo", "stopped", "stopped"), withUrl, done, fullTask("a1", "delta", "archived", "stopped")],
      orphans: [],
      clones_bytes: null,
    };
    localStorage.setItem("fleet.product", "p");
    const p = product("p", "p", {
      repos: [repoInfo("core", { product: "p", url: null }), repoInfo("web", { product: "p", url: "https://git.example/web" })],
    });
    const { client } = await tasks("s1", list, products(p));
    expect(await passport().findByRole("button", { name: "Mark done" })).toBeInTheDocument();
    for (const [id, filter] of [
      ["u1", "all"],
      ["d1", "all"],
      ["a1", "dropped"],
    ] as const) {
      go(tasksHref(filter, id));
      expect(passport().queryByRole("button", { name: "Mark done" }), id).toBeNull();
    }
    go(tasksHref("all", "s1"));
    await userEvent.click(button("Mark done"));
    expect(client.markDone).toHaveBeenCalledWith("s1");
  });

  it("a_failed_change_says_so_in_the_footer", async () => {
    localStorage.setItem("fleet.product", "p");
    const markDone = vi.fn().mockRejectedValue(new Error("refused"));
    const { tasks: fetch } = await tasks("s1", undefined, { markDone, ...products(productP) });
    const before = fetch.mock.calls.length;
    await userEvent.click(await passport().findByRole("button", { name: "Mark done" }));
    expect(await screen.findByText("Could not update bravo: refused")).toBeInTheDocument();
    // Either way the list is asked for again.
    await waitFor(() => expect(fetch.mock.calls.length).toBeGreaterThan(before));
  });

  it("moving_release_calls_client", async () => {
    const [list, over] = withProducts([fullTask("s1", "bravo", "stopped", "stopped")], "p");
    const { client } = await tasks("s1", list, over);
    await userEvent.click(await passport().findByRole("button", { name: "Release" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "2.0" }));
    expect(client.setTaskRelease).toHaveBeenCalledWith("s1", 2);
    expect(screen.queryByRole("menu"), "a pick closes the menu").toBeNull();
    // The release it is in already: nothing to move.
    await userEvent.click(passport().getByRole("button", { name: "Release" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Backlog" }));
    expect(client.setTaskRelease).not.toHaveBeenCalledWith("s1", null);
  });

  it("an_arrow_key_on_the_closed_release_control_moves_nothing", async () => {
    // egui's combo acts only on a click in its open popup; a native select would move the task on ArrowDown.
    const [list, over] = withProducts([fullTask("s1", "bravo", "stopped", "stopped")], "p");
    const { client } = await tasks("s1", list, over);
    (await passport().findByRole("button", { name: "Release" })).focus();
    await userEvent.keyboard("{ArrowDown}{ArrowUp}{ArrowDown}");
    expect(client.setTaskRelease).not.toHaveBeenCalled();
  });

  it("a_finished_task_cannot_change_release", async () => {
    const [list, over] = withProducts([fullTask("s1", "bravo", "stopped", "stopped", { done_by_hand: true })], "p");
    await tasks("s1", list, over);
    const release = passport().getByRole("button", { name: "Release" });
    expect(release).toBeDisabled();
    expect(release).toHaveAttribute("title", "A finished task stays in its release");
  });

  it("an_open_pr_disables_archive_with_its_reason", async () => {
    const pr = { url: "https://git.example/core/pull/1", repo: "core", state: "open" as const, merged_at: null };
    const [list, over] = withProducts([fullTask("s1", "bravo", "stopped", "stopped", { prs: [pr] })], "p");
    await tasks("s1", list, over);
    expect(button("Archive…")).toBeDisabled();
    expect(passport().getByText("https://git.example/core/pull/1")).toBeInTheDocument();
    expect(screen.getAllByText("Open").length, "badge and PR state").toBeGreaterThanOrEqual(2);
    expect(button("Archive…")).toHaveAttribute(
      "title",
      "https://git.example/core/pull/1 is still open; merge or close it before archiving",
    );
  });

  it("mark_done_is_hidden_before_products_load", async () => {
    // The saved product is trusted before the list arrives: its repos are unknown, not URL-less.
    localStorage.setItem("fleet.product", "p");
    await tasks("s1", undefined, { products: vi.fn(() => new Promise(() => {})) });
    expect(passport().queryByRole("button", { name: "Mark done" })).toBeNull();
  });

  it("a_released_release_is_not_offered", async () => {
    localStorage.setItem("fleet.product", "p");
    const p = product("p", "p", {
      releases: [
        { id: 1, product: "p", version: "1.0", start: "2026-10-01", target: "2026-11-01", released_at: null },
        { id: 3, product: "p", version: "0.9", start: "2026-10-01", target: "2026-11-01", released_at: NOW - 1000 },
      ],
    });
    await tasks("s1", undefined, products(p));
    await userEvent.click(passport().getByRole("button", { name: "Release" }));
    expect(await screen.findByRole("menuitem", { name: "1.0" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /0\.9/ })).toBeNull();
  });

  it("a_task_in_a_released_release_keeps_it_on_show", async () => {
    localStorage.setItem("fleet.product", "p");
    const p = product("p", "p", {
      releases: [{ id: 3, product: "p", version: "0.9", start: "2026-10-01", target: "2026-11-01", released_at: NOW - 1000 }],
    });
    await tasks(
      "s1",
      { tasks: [fullTask("s1", "bravo", "stopped", "stopped", { release: 3 })], orphans: [], clones_bytes: null },
      products(p),
    );
    const release = passport().getByRole("button", { name: "Release" });
    await waitFor(() => expect(release).toHaveTextContent("0.9 (released)"));
  });

  it("the_passport_sections_are_mono_labels", async () => {
    await tasks("s1");
    for (const s of ["Goal", "Repos"]) expect(passport().getByText(s)).toHaveClass("mono-label");
  });

  it("an_orphans_passport_says_what_it_is", async () => {
    const list = tasksList();
    list.orphans[0]!.worktrees = ["/p/echo/core"];
    await tasks("task_folder:/p/echo", list);
    expect(passport().getByText("echo")).toBeInTheDocument();
    expect(passport().getByText("Orphan")).toHaveAttribute("title", "Found with no task record");
    expect(passport().getByText("A folder under the tasks root with no task record")).toBeInTheDocument();
    expect(passport().getByText("/p/echo/core")).toBeInTheDocument();
    expect(passport().getByText("1.0 MB")).toBeInTheDocument();
    expect(button("Archive…")).toBeEnabled();
  });

  it("the_browser_offers_every_tasks_action", async () => {
    // No native bridge: every Tasks action is a daemon call, so none is greyed in a browser.
    const [list, over] = withProducts([fullTask("s1", "bravo", "stopped", "stopped"), fullTask("r1", "alpha", "running", null)], "p");
    await tasks("s1", list, over);
    for (const name of ["Archive…", "Open on board", "Mark done", "Refresh", "Reload the diff"])
      expect(await passport().findByRole("button", { name })).toBeEnabled();
    expect(passport().getByRole("button", { name: "Release" })).toBeEnabled();
    onBoard();
    go(tasksHref("all", "r1"));
    expect(await passport().findByRole("button", { name: "Stop" })).toBeEnabled();
  });
});
