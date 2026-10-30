import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FleetError } from "../api/client";
import type { Release } from "../api/generated/Release";
import type { Task } from "../api/generated/Task";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, factory, fakeClient, fullTask, persona, product, schema } from "../test/fixtures";

// Ports of crates/fleet-gui/tests/releases.rs.

const css = readFileSync("src/releases/releases.css", "utf8");

/** Local noon of `date`, so the local day is `date` wherever the tests run. */
function noon(date: string) {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return new Date(y, m - 1, d, 12).getTime() / 1000;
}

const release = (id: number, version: string, start: string, target: string, released_at: number | null = null): Release => ({
  id,
  product: "p",
  version,
  start,
  target,
  released_at,
});

const inRelease = (id: string, release: number | null) => fullTask(id, `task-${id}`, "stopped", "stopped", { release });

function merged(id: string, release: number | null): Task {
  const t = fullTask(id, `task-${id}`, "archived", "stopped", {
    release,
    prs: [{ url: "u", repo: "core", state: "merged", merged_at: null }],
  });
  t.repos[0]!.changed = true;
  return t;
}

/** egui's `releases_view`: product "p" picked, these releases and tasks, the clock at noon of `today`. */
function releasesView(today: string, releases: Release[], tasks: Task[], over: Parameters<typeof fakeClient>[0] = {}) {
  localStorage.setItem("fleet.product", "p");
  window.location.hash = "#releases";
  const products = vi.fn().mockResolvedValue({ products: [product("p", "p", { releases })] });
  const client = fakeClient({
    personas: vi.fn().mockResolvedValue({ personas: [persona("p")] }),
    products,
    tasks: vi.fn().mockResolvedValue({ tasks, orphans: [], clones_bytes: null }),
    submitForm: vi.fn().mockResolvedValue({}),
    ...over,
  });
  render(<App client={client} eventSource={factory} nowSeconds={() => noon(today)} />);
  return { client, products };
}

const card = (version: string) => screen.getByRole("article", { name: version });

beforeEach(() => {
  FakeEventSource.reset();
  localStorage.clear();
});

describe("Releases view", () => {
  it("bars_match_release_progress", async () => {
    releasesView(
      "2026-10-11",
      [release(1, "1.0", "2026-10-01", "2026-11-01")],
      [merged("a", 1), fullTask("b", "task-b", "archived", "stopped", { release: 1 }), inRelease("c", 1)],
    );
    expect(await screen.findByText("1/2 tasks")).toBeInTheDocument();
    expect(screen.getByText("11/32 days")).toBeInTheDocument();
    expect(within(card("1.0")).getByText("1.0")).toBeInTheDocument();
    expect(within(card("1.0")).getByText("2026-10-01 to 2026-11-01")).toBeInTheDocument();
    // Its tasks are listed.
    expect(within(card("1.0")).getByText("task-a")).toBeInTheDocument();
    expect(within(card("1.0")).getByText("task-c")).toBeInTheDocument();
  });

  it("overdue_label_past_target", async () => {
    releasesView("2026-11-06", [release(1, "1.0", "2026-10-01", "2026-11-01")], []);
    expect(await screen.findByText("5 days overdue")).toHaveAttribute("data-tone", "attention");
  });

  it("bad_dates_show_daemon_error", async () => {
    const submitForm = vi.fn().mockRejectedValue(new FleetError("target is not a date (YYYY-MM-DD)"));
    const { client } = releasesView("2026-10-11", [], [], { submitForm });
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Version" }), "3.0");
    await userEvent.type(screen.getByRole("textbox", { name: "Target" }), "soon");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(client.submitForm).toHaveBeenCalledWith(
      expect.objectContaining({ id: "release_create" }),
      expect.objectContaining({ id: "create" }),
      { product: "p" },
      { version: "3.0", start: "2026-10-11", target: "soon" },
    );
    expect(await screen.findByText("target is not a date (YYYY-MM-DD)")).toBeInTheDocument();
    // The form stays open for a retry.
    expect(screen.getByRole("textbox", { name: "Target" })).toBeInTheDocument();
  });

  it("a_saved_release_closes_the_form_and_reloads_the_products", async () => {
    const { products } = releasesView("2026-10-11", [], []);
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Version" }), "3.0");
    await userEvent.type(screen.getByRole("textbox", { name: "Target" }), "2026-12-01");
    const before = products.mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Version" })).toBeNull());
    expect(products.mock.calls.length).toBeGreaterThan(before);
    await userEvent.click(screen.getByRole("button", { name: "New release" }));
    // egui's Create is its primary button.
    expect(screen.getByRole("button", { name: "Create" })).toHaveAttribute("data-variant", "primary");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox", { name: "Version" })).toBeNull();
  });

  it("mark_released_moves_it_down", async () => {
    const two = release(2, "2.0", "2026-09-01", "2026-10-01");
    const { client, products } = releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01"), two], []);
    await screen.findByRole("article", { name: "1.0" });
    const versions = () => screen.getAllByRole("article").map((a) => a.getAttribute("aria-label"));
    expect(versions(), "newest start first").toEqual(["1.0", "2.0"]);
    products.mockResolvedValue({
      products: [product("p", "p", { releases: [release(1, "1.0", "2026-10-15", "2026-11-01", noon("2026-10-26")), two] })],
    });
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Mark released" }));
    expect(client.updateRelease).toHaveBeenCalledWith(1, { released: true });
    await waitFor(() => expect(versions(), "released goes after unreleased").toEqual(["2.0", "1.0"]));
    expect(within(card("1.0")).getByText("Released 2026-10-26")).toBeInTheDocument();
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Unrelease" }));
    expect(client.updateRelease).toHaveBeenLastCalledWith(1, { released: false });
  });

  it("delete_is_confirmed_in_the_page", async () => {
    const { client } = releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01")], []);
    await userEvent.click(await within(await screen.findByRole("article", { name: "1.0" })).findByRole("button", { name: "Delete" }));
    expect(within(card("1.0")).getByText("Delete 1.0? Its tasks go to the backlog")).toBeInTheDocument();
    expect(client.deleteRelease).not.toHaveBeenCalled();
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Cancel" }));
    expect(within(card("1.0")).queryByText(/Its tasks go to the backlog/)).toBeNull();
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Delete" }));
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Confirm" }));
    expect(client.deleteRelease).toHaveBeenCalledWith(1);
  });

  it("moving_a_task_and_finished_tasks_are_locked", async () => {
    const { client } = releasesView(
      "2026-10-26",
      [release(1, "1.0", "2026-10-15", "2026-11-01"), release(2, "2.0", "2026-09-01", "2026-10-01")],
      [inRelease("c", 1), merged("m", 2)],
    );
    await screen.findByText("task-c");
    const combos = screen.getAllByRole("button", { name: "Move to…" });
    expect(combos).toHaveLength(2);
    expect(combos[0]).toBeEnabled();
    expect(combos[1]).toBeDisabled();
    expect(combos[1]).toHaveAttribute("title", "A finished task stays in its release");
    // Keys on the closed control move nothing: only a pick in its open menu does, as egui's combo.
    combos[0]!.focus();
    await userEvent.keyboard("{ArrowDown}{ArrowUp}");
    expect(client.setTaskRelease).not.toHaveBeenCalled();
    await userEvent.click(combos[0]!);
    const menu = screen.getByRole("menu");
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((b) => b.textContent),
    ).toEqual(["2.0", "Backlog"]);
    await userEvent.click(within(menu).getByRole("menuitem", { name: "2.0" }));
    expect(client.setTaskRelease).toHaveBeenCalledWith("c", 2);
    expect(screen.queryByRole("menu"), "a pick closes it").toBeNull();
  });

  it("no_product_says_pick_one", async () => {
    window.location.hash = "#releases";
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => noon("2026-10-26")} />);
    // The drawer's product combo says it too.
    expect(await within(screen.getByRole("main")).findByText("Pick a product")).toBeInTheDocument();
  });

  it("edit_prefills_the_release_not_today", async () => {
    const { client } = releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01")], []);
    await userEvent.click(await within(await screen.findByRole("article", { name: "1.0" })).findByRole("button", { name: "Edit" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(client.submitForm).toHaveBeenCalledWith(
      expect.objectContaining({ id: "release_edit" }),
      expect.objectContaining({ id: "save" }),
      { id: "1" },
      { version: "1.0", start: "2026-10-15", target: "2026-11-01" },
    );
  });

  it("the_form_follows_the_daemons_description", async () => {
    const changed = structuredClone(schema) as UiSchema;
    const form = changed.forms.find((f) => f.id === "release_create")!;
    form.title = "New version";
    const group = form.items[0]!;
    if (group.type !== "group") throw new Error("the release form starts with a group");
    group.fields[0]!.label = "Number";
    releasesView("2026-10-26", [], [], { uiSchema: vi.fn().mockResolvedValue(changed) });
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    expect(screen.getByText("New version")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Number" })).toBeInTheDocument();
  });

  it("a_refusal_lands_under_its_field", async () => {
    const submitForm = vi.fn().mockRejectedValue(new FleetError('"soon" is not a date; use YYYY-MM-DD', "target"));
    releasesView("2026-10-11", [], [], { submitForm });
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Version" }), "3.0");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    const msg = await screen.findByText('"soon" is not a date; use YYYY-MM-DD');
    expect(msg.closest(".form-input")).toHaveAttribute("data-key", "target");
  });

  it("a_refusal_for_another_field_stays_at_form_level", async () => {
    const submitForm = vi.fn().mockRejectedValue(new FleetError('"soon" is not a date; use YYYY-MM-DD', "owner"));
    releasesView("2026-10-11", [], [], { submitForm });
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Version" }), "3.0");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    const msg = await screen.findByText('"soon" is not a date; use YYYY-MM-DD');
    expect(msg.closest(".form-input")).toBeNull();
  });

  it("no_releases_says_so", async () => {
    releasesView("2026-10-11", [], []);
    expect(await screen.findByText("No releases")).toBeInTheDocument();
  });

  it("a_refused_release_change_tells_the_footer", async () => {
    const updateRelease = vi.fn().mockRejectedValue(new FleetError("nope"));
    const { products } = releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01")], [], { updateRelease });
    const before = await waitFor(() => {
      expect(products).toHaveBeenCalled();
      return products.mock.calls.length;
    });
    await userEvent.click(
      await within(await screen.findByRole("article", { name: "1.0" })).findByRole("button", { name: "Mark released" }),
    );
    expect(await screen.findByText("Could not update the release: nope")).toBeInTheDocument();
    await waitFor(() => expect(products.mock.calls.length).toBeGreaterThan(before));
  });

  it("a_refused_move_names_the_task", async () => {
    const setTaskRelease = vi.fn().mockRejectedValue(new FleetError("nope"));
    releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01")], [inRelease("c", 1)], { setTaskRelease });
    await screen.findByText("task-c");
    await userEvent.click(screen.getByRole("button", { name: "Move to…" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Backlog" }));
    expect(setTaskRelease).toHaveBeenCalledWith("c", null);
    expect(await screen.findByText("Could not update task-c: nope")).toBeInTheDocument();
  });

  it("a_refetch_keeps_the_open_form", async () => {
    const { products } = releasesView("2026-10-11", [release(1, "1.0", "2026-10-01", "2026-11-01")], []);
    await userEvent.click(await screen.findByRole("button", { name: "New release" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Version" }), "3.0");
    products.mockResolvedValue({
      products: [
        product("p", "p", { releases: [release(1, "1.0", "2026-10-01", "2026-11-01"), release(2, "2.0", "2026-10-01", "2026-11-01")] }),
      ],
    });
    // Another release changes: the list is fetched again under the open form.
    await userEvent.click(within(card("1.0")).getByRole("button", { name: "Mark released" }));
    await screen.findByRole("article", { name: "2.0" });
    expect(screen.getByRole("textbox", { name: "Version" })).toHaveValue("3.0");
  });

  it("the_releases_css_scrolls_and_uses_tokens_only", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).toMatch(/\.releases \{[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.release-task-name \{[^}]*text-overflow: ellipsis/);
  });

  it("the_browser_offers_every_releases_action", async () => {
    releasesView("2026-10-26", [release(1, "1.0", "2026-10-15", "2026-11-01")], [inRelease("c", 1)]);
    const c = await screen.findByRole("article", { name: "1.0" });
    await within(c).findByText("task-c");
    expect(screen.getByRole("button", { name: "New release" })).toBeEnabled();
    for (const name of ["Mark released", "Edit", "Delete"]) expect(within(c).getByRole("button", { name })).toBeEnabled();
    expect(within(c).getByRole("button", { name: "Move to…" })).toBeEnabled();
  });

  it("the_drawer_opens_releases", async () => {
    releasesView("2026-10-26", [], []);
    const link = within(await screen.findByRole("navigation", { name: "Drawer" })).getByRole("link", { name: "Releases" });
    expect(link).toHaveAttribute("aria-current", "page");
    expect(within(screen.getByRole("banner")).getByRole("heading", { name: "Releases" })).toBeInTheDocument();
    await act(async () => {});
  });
});
