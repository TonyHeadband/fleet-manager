import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../api/generated/Task";
import { FakeEventSource, agent, fullTask, persona, tasksList } from "../test/fixtures";
import { layout } from "../generated/layout";
import { tasksHref } from "../shell/routes";
import { go, row, rows, snapshot, start, tasks, totals, withProducts } from "./harness";

// Ports of crates/fleet-gui/tests/tasks.rs (the list, the totals and the header) and of the board's ways into the
// view from tests/app.rs.

const css = readFileSync("src/tasks/tasks.css", "utf8");

beforeEach(() => {
  FakeEventSource.reset();
  localStorage.clear();
  window.location.hash = "";
});
afterEach(() => vi.useRealTimers());

describe("Tasks view", () => {
  it("the_drawer_lists_tasks_between_kanban_and_settings", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    const links = within(drawer)
      .getAllByRole("link")
      .map((l) => l.textContent);
    expect(links.indexOf("Kanban")).toBeLessThan(links.indexOf("Tasks"));
    expect(links.indexOf("Tasks")).toBeLessThan(links.indexOf("Settings"));
    const link = within(drawer).getByRole("link", { name: "Tasks" });
    expect(link).not.toHaveAttribute("aria-disabled");
    await userEvent.click(link);
    expect(within(screen.getByRole("banner")).getByRole("heading", { name: "Tasks" })).toBeInTheDocument();
    expect(within(drawer).getByRole("link", { name: "Tasks" })).toHaveAttribute("aria-current", "page");
  });

  it("the_list_is_narrower_than_the_passport", () => {
    // egui's LIST_WIDTH, one token for both apps; the passport takes what is left.
    expect(layout.taskList).toBe(250);
    expect(css).toMatch(/\.tasks-list \{[^}]*flex: none/);
  });

  it("totals_count_crashed_on_their_own", async () => {
    await tasks("r1");
    for (const text of ["1 running", "2 stopped", "1 crashed", "1 orphan", "2.5 GB"])
      expect(within(totals()).getByText(text)).toBeInTheDocument();
    // Held: three live tasks and the orphan; reclaimable: the two stopped ones and the orphan.
    expect(within(totals()).getByText("24.0 GB held")).toBeInTheDocument();
    expect(within(totals()).getByText("16.0 GB reclaimable")).toBeInTheDocument();
  });

  it("the_list_orders_running_then_crashed_then_stopped_then_orphans", async () => {
    await tasks("r1");
    expect(rows()).toEqual(["alpha", "charlie", "bravo", "echo"]);
  });

  it("archived_tasks_are_hidden_until_show_archived", async () => {
    await tasks("r1");
    expect(rows()).not.toContain("delta");
    const toggle = within(screen.getByRole("banner")).getByRole("button", { name: "Show archived" });
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(rows()).toEqual(["alpha", "charlie", "bravo", "echo", "delta"]);
    // Archived rows are dimmed, as egui's 0.55 opacity.
    expect(row("delta")).toHaveAttribute("data-archived", "true");
  });

  it("open_tasks_filters_the_list", async () => {
    window.location.hash = tasksHref("crashed");
    start();
    await screen.findByRole("list", { name: "Tasks" });
    expect(screen.getByText("Showing crashed")).toBeInTheDocument();
    expect(rows()).toEqual(["charlie"]);
    await userEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(rows()).toContain("bravo");
    expect(screen.queryByText(/^Showing/)).toBeNull();
  });

  it("task_row_shows_persona", async () => {
    const l = tasksList();
    // bravo's persona was deleted since; charlie predates personas.
    l.tasks[1]!.persona = "gone";
    l.tasks[2]!.persona = "";
    await tasks("r1", l, {
      personas: vi
        .fn()
        .mockResolvedValue({ personas: [persona("p"), persona("w")].map((x, i) => ({ ...x, name: ["Personal", "Work"][i] })) }),
    });
    await waitFor(() => expect(within(row("alpha")).getByText("Personal")).toBeInTheDocument());
    expect(within(row("bravo")).getByText("gone")).toBeInTheDocument();
    expect(within(row("charlie")).getByText("no persona")).toBeInTheDocument();
  });

  it("task_rows_tell_same_named_personas_apart", async () => {
    const l = tasksList();
    l.tasks[1]!.persona = "w";
    await tasks("r1", l, {
      personas: vi.fn().mockResolvedValue({
        personas: [
          { ...persona("p"), name: "Ann" },
          { ...persona("w"), name: "Ann" },
        ],
      }),
    });
    await waitFor(() => expect(within(row("alpha")).getByText("Ann (p)")).toBeInTheDocument());
    expect(within(row("bravo")).getByText("Ann (w)")).toBeInTheDocument();
  });

  it("lists_only_active_products_tasks", async () => {
    const [list, over] = withProducts(
      [fullTask("r1", "alpha", "running", null), fullTask("o1", "zulu", "stopped", "stopped", { product: "q" })],
      "p",
    );
    await tasks("r1", list, over);
    await waitFor(() => expect(rows()).not.toContain("zulu"));
    expect(rows()).toContain("alpha");
    // The totals count the product's tasks only.
    expect(within(totals()).getByText("1 running")).toBeInTheDocument();
    expect(within(totals()).getByText("0 stopped")).toBeInTheDocument();
  });

  it("orphans_show_for_any_product", async () => {
    const [list, over] = withProducts([fullTask("r1", "alpha", "running", null)], "q");
    await tasks("r1", list, over);
    await waitFor(() => expect(rows()).toEqual(["echo"]));
  });

  it("merged_filter", async () => {
    const merged = fullTask("m1", "mike", "archived", "stopped", {
      prs: [{ url: "https://git.example/core/pull/1", repo: "core", state: "merged", merged_at: null }],
    });
    merged.repos[0]!.changed = true;
    const [list, over] = withProducts([merged, fullTask("s1", "bravo", "stopped", "stopped")], "p");
    window.location.hash = tasksHref("merged");
    start(list, over);
    await screen.findByRole("list", { name: "Tasks" });
    expect(rows()).toEqual(["mike"]);
    expect(within(row("mike")).getByText("Merged")).toBeInTheDocument();
  });

  it("dropped_after_archive_shows_dropped", async () => {
    const [list, over] = withProducts([fullTask("a1", "delta", "archived", "stopped")], "p");
    window.location.hash = tasksHref("dropped");
    start(list, over);
    await screen.findByRole("list", { name: "Tasks" });
    expect(rows()).toEqual(["delta"]);
    expect(within(row("delta")).getByText("Dropped")).toBeInTheDocument();
  });

  it("an_orchestrated_row_shows_its_steps", async () => {
    const step = (n: number, state: "done" | "pending") => ({
      n,
      title: `step ${n}`,
      specialist: "rust",
      brief: "do it",
      depends_on: [],
      release_note: false,
      state,
      agent: null,
      branch: null,
      report: null,
    });
    const t = fullTask("o1", "oscar", "running", null, {
      kind: "orchestrator",
      breakdowns: [
        {
          version: 1,
          steps: [step(1, "done"), step(2, "done"), step(3, "pending")],
          approval: "approved",
          comment: null,
          proposed_at: 1,
          decided_at: 2,
        },
      ],
    } as Partial<Task>);
    await tasks("o1", { tasks: [t], orphans: [], clones_bytes: null });
    expect(within(row("oscar")).getByText("2/3 steps")).toBeInTheDocument();
  });

  it("the_stat_captions_are_mono_uppercase_and_keep_their_labels", async () => {
    await tasks("s1");
    const caption = within(totals()).getByText("Agents started");
    expect(caption).toHaveClass("mono-label");
    expect(within(totals()).getByText("4")).toBeInTheDocument();
  });

  it("a_running_row_wears_its_board_column_and_a_stopped_one_its_cause", async () => {
    await tasks("r1");
    snapshot(agent({ id: "r1", status: "needs_you" }));
    await waitFor(() => expect(within(row("alpha")).getByText("Needs you")).toHaveAttribute("data-tone", "attention"));
    expect(within(row("charlie")).getByText("Crashed")).toHaveAttribute("data-tone", "danger");
    expect(within(row("bravo")).getByText("Stopped")).toHaveAttribute("data-tone", "caution");
    expect(within(row("echo")).getByText("Orphan")).toHaveAttribute("data-tone", "caution");
    expect(within(row("echo")).getByText("1.0 MB")).toBeInTheDocument();
  });

  it("the_release_filter_narrows_the_list_as_on_the_board", async () => {
    const [list, over] = withProducts(
      [fullTask("r1", "alpha", "running", null, { release: 1 }), fullTask("s1", "bravo", "stopped", "stopped")],
      "p",
    );
    await tasks("r1", list, over);
    await userEvent.click(within(screen.getByRole("banner")).getByRole("button", { name: "Release filter" }));
    await userEvent.click(screen.getByRole("button", { name: "1.0" }));
    expect(rows()).toEqual(["alpha", "echo"]);
  });

  it("no_tasks_says_so", async () => {
    await tasks("x", { tasks: [], orphans: [], clones_bytes: null });
    expect(screen.getByText("No tasks")).toBeInTheDocument();
    expect(screen.getByText("Nothing to show")).toBeInTheDocument();
  });

  it("a_selected_task_that_leaves_the_list_moves_to_the_first_row", async () => {
    await tasks("s1");
    expect(row("bravo")).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(row("charlie"));
    expect(row("charlie")).toHaveAttribute("aria-pressed", "true");
    expect(window.location.hash).toBe(tasksHref(undefined, "c1"));
    // Filtered out: the first row the list still shows takes over, and the address follows.
    go(tasksHref("running", "c1"));
    expect(row("alpha")).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(window.location.hash).toBe(tasksHref("running", "r1")));
    expect(within(screen.getByRole("region", { name: "Passport" })).getByText("alpha")).toBeInTheDocument();
  });

  it("a_failed_refetch_keeps_the_list_and_says_so", async () => {
    let fail = false;
    const { tasks: fetch } = await tasks("r1", undefined, {
      tasks: vi.fn(async () => {
        if (fail) throw new Error("daemon gone");
        return tasksList();
      }),
    });
    void fetch;
    fail = true;
    snapshot(agent({ id: "r1" }));
    expect(await within(totals()).findByText("Could not refresh: daemon gone")).toBeInTheDocument();
    expect(rows()).toEqual(["alpha", "charlie", "bravo", "echo"]);
    // As egui: Retry is for a view with nothing to show; the poll keeps trying here.
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("a_first_load_that_fails_offers_retry", async () => {
    let fail = true;
    window.location.hash = tasksHref();
    start(async () => {
      if (fail) throw new Error("daemon gone");
      return tasksList();
    });
    expect(await screen.findByText("Cannot load the tasks: daemon gone")).toBeInTheDocument();
    fail = false;
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("list", { name: "Tasks" })).toBeInTheDocument();
  });

  it("the_tasks_css_scrolls_in_its_boxes_and_uses_tokens_only", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    // The list is egui's fixed 250 and the passport takes the rest: the list is the narrower.
    expect(css).toMatch(/\.tasks-list \{[^}]*width: var\(--layout-task-list\)/);
    expect(css).toMatch(/\.tasks-list \{[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.passport \{[^}]*flex: 1/);
    expect(css).toMatch(/\.passport \{[^}]*overflow-y: auto/);
  });
});

describe("The board's ways into Tasks", () => {
  const strip = {
    tasks: [fullTask("r1", "proj-r1", "running", null), fullTask("s1", "proj-s1", "stopped", "stopped")],
    orphans: tasksList().orphans,
    clones_bytes: null,
  };

  it("leaving_tasks_keeps_its_filter_and_selection", async () => {
    window.location.hash = tasksHref("stopped", "s1");
    start();
    await screen.findByRole("list", { name: "Tasks" });
    expect(row("bravo")).toHaveAttribute("aria-pressed", "true");
    const drawer = screen.getByRole("navigation", { name: "Drawer" });
    await userEvent.click(within(drawer).getByRole("link", { name: "Kanban" }));
    // Back through the drawer: egui's TasksState is kept while the view is away.
    await userEvent.click(within(drawer).getByRole("link", { name: "Tasks" }));
    expect(await screen.findByText("Showing stopped")).toBeInTheDocument();
    expect(row("bravo")).toHaveAttribute("aria-pressed", "true");
    // A strip count changes the filter and keeps a selection the filtered list still shows (`open_tasks(f, None)`).
    await userEvent.click(within(drawer).getByRole("link", { name: "Kanban" }));
    await userEvent.click(await within(screen.getByRole("region", { name: "Tasks strip" })).findByText("1 crashed"));
    expect(await screen.findByText("Showing crashed")).toBeInTheDocument();
    expect(row("charlie")).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(within(drawer).getByRole("link", { name: "Kanban" }));
    await userEvent.click(within(screen.getByRole("region", { name: "Tasks strip" })).getByText("2 stopped"));
    expect(await screen.findByText("Showing stopped")).toBeInTheDocument();
    expect(row("charlie")).toHaveAttribute("aria-pressed", "true");
  });

  it("header_counts_show_crashed_and_orphans_only_when_present", async () => {
    start({ ...strip, orphans: [] });
    const s = await screen.findByRole("region", { name: "Tasks strip" });
    await within(s).findByText("1 running");
    expect(within(s).queryByText(/crashed/)).toBeNull();
    expect(within(s).queryByText(/orphan/)).toBeNull();
  });

  it("clicking_stopped_opens_tasks_filtered_to_stopped", async () => {
    start(strip);
    const s = await screen.findByRole("region", { name: "Tasks strip" });
    await userEvent.click(await within(s).findByText("1 stopped"));
    expect(await screen.findByText("Showing stopped")).toBeInTheDocument();
    expect(rows()).toEqual(["proj-s1"]);
    await userEvent.click(within(screen.getByRole("navigation", { name: "Drawer" })).getByRole("link", { name: "Kanban" }));
    await userEvent.click(await within(screen.getByRole("region", { name: "Tasks strip" })).findByText("1 orphan"));
    expect(await screen.findByText("Showing orphans")).toBeInTheDocument();
  });

  it("the_strip_counts_name_the_filter_they_open", async () => {
    start({ ...strip, tasks: [...strip.tasks, fullTask("c1", "proj-c1", "stopped", "crashed")] });
    const s = await screen.findByRole("region", { name: "Tasks strip" });
    expect((await within(s).findByText("1 orphan")).closest("a")).toHaveAttribute("title", "Show orphans");
    expect(within(s).getByText("1 crashed").closest("a")).toHaveAttribute("title", "Show crashed tasks");
    expect(within(s).getByText("1 running").closest("a")).toHaveAttribute("href", tasksHref("running"));
  });

  it("details_on_a_stopped_card_selects_it_in_tasks", async () => {
    start(strip);
    snapshot(agent({ id: "s1", goal: "fix login", status: "stopped", stop_cause: "stopped" }), agent({ id: "r1", goal: "running one" }));
    // Only the stopped card offers it.
    const details = await screen.findAllByRole("button", { name: "Details" });
    expect(details).toHaveLength(1);
    await userEvent.click(details[0]!);
    expect(await screen.findByRole("list", { name: "Tasks" })).toBeInTheDocument();
    expect(row("proj-s1")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText("Showing stopped"), "Details shows every task").toBeNull();
  });

  it("the_board_polls_the_tasks_list_for_its_counts", async () => {
    const { tasks: fetch } = start(strip);
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("heading", { name: "Kanban" })).toBeInTheDocument();
  });
});
