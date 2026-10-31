import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { Product } from "../api/generated/Product";
import type { ProductOverride } from "../api/generated/ProductOverride";
import { click, managedDef as def, managedProduct as product, managedRepo as repo, management, pending } from "./harness";

// Ports of the Products tests in crates/fleet-gui/tests/management.rs.

const alpha = (over: Partial<Product> = {}) => product("p", "Alpha", [], over);

async function products(list: Product[], over = {}) {
  const client = await management("Products", { products: list, over });
  await screen.findByRole("heading", { name: list[0]?.name ?? "" }).catch(() => {});
  return client;
}

const field = (name: string) => screen.getByRole("textbox", { name });
const button = (name: string) => screen.getByRole("button", { name });

beforeEach(() => localStorage.clear());

describe("Products: the form", () => {
  it("products_row_shows_workflow_state", async () => {
    const cases: [Product["repos"][number]["workflow"], string][] = [
      [{ state: "found", base: "main", steps: ["fmt", "gui"] }, "Checks on main: fmt, gui"],
      [
        { state: "missing", base: "main" },
        "fleet has no .fleet/workflow.yml on main. It lists the checks the gate runs before a pull request, so orchestrated tasks need it.",
      ],
      [{ state: "invalid", base: "main", error: "bad" }, "fleet: .fleet/workflow.yml on main is invalid: bad"],
      [{ state: "unreadable", base: "main", error: "no origin" }, "fleet: cannot read .fleet/workflow.yml on main: no origin"],
    ];
    for (const [workflow, text] of cases) {
      const { unmount } = await renderProducts([alpha({ repos: [repo("fleet", "/p/fleet", null, { workflow })] })]);
      expect(await screen.findByText(text)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Create workflow" })).toBeNull();
      unmount();
    }
  });

  it("repo_row_lists_its_env", async () => {
    const env = [
      { name: "CARGO_HOME", value: "/c", source: "fleet", gate: false, overridden: true },
      { name: "CARGO_HOME", value: "{task}/cargo", source: "repo", gate: true, overridden: false },
      { name: "RUSTFLAGS", value: "-Dwarnings", source: "fleet", gate: false, overridden: false },
    ] as const;
    await products([alpha({ repos: [repo("fleet", "/p/fleet", null, { env: [...env] })] })]);
    for (const text of [
      "CARGO_HOME = {task}/cargo · repo",
      "CARGO_HOME = /c · fleet, overridden by the repo",
      "RUSTFLAGS = -Dwarnings · fleet, not in the gate",
    ])
      expect(await screen.findByText(text)).toBeInTheDocument();
  });

  it("products_tab_edits_repo_url", async () => {
    const client = await products([
      alpha({ repos: [repo("core", "/src/core", null), repo("rest", null, "https://git.example/rest.git")] }),
    ]);
    await userEvent.type(field("Name"), "!");
    await userEvent.type(field("Clone URL 1"), "https://git.example/core.git");
    await click("Remove repo 2");
    await click("Add repo");
    await userEvent.type(field("Repo name 2"), "docs");
    await userEvent.type(field("Local path 2"), "/src/docs");
    await click("Save");
    await waitFor(() => expect(client.deleteRepo).toHaveBeenCalled());
    expect(client.updateProduct).toHaveBeenCalledWith("p", { name: "Alpha!" });
    expect(vi.mocked(client.upsertRepo).mock.calls).toEqual([
      ["p", { name: "core", path: null, url: "https://git.example/core.git" }],
      ["p", { name: "docs", path: "/src/docs", url: null }],
    ]);
    expect(client.deleteRepo).toHaveBeenCalledWith("p", "rest");
  });

  it("products_tab_sets_the_internal_url", async () => {
    const client = await products([alpha()]);
    expect(screen.getByText("HTTPS URL. Cloned with the default persona's GitHub login; tasks push their notes here.")).toBeInTheDocument();
    await userEvent.type(field("Internal repo URL"), "https://git.example/notes.git");
    await click("Save");
    expect(client.updateProduct).toHaveBeenCalledWith("p", { name: null, internal_url: "https://git.example/notes.git" });
  });

  it("products_tab_shows_the_stored_internal_url_and_leaves_it_alone_when_unchanged", async () => {
    const client = await products([alpha({ internal_url: "https://git.example/notes.git" })]);
    expect(field("Internal repo URL")).toHaveValue("https://git.example/notes.git");
    await userEvent.type(field("Name"), "!");
    await click("Save");
    expect(client.updateProduct).toHaveBeenCalledWith("p", { name: "Alpha!" });
  });

  it("products_tab_clears_a_set_internal_url_with_null", async () => {
    const client = await products([alpha({ internal_url: "https://git.example/notes.git" })]);
    await userEvent.clear(field("Internal repo URL"));
    await click("Save");
    expect(client.updateProduct).toHaveBeenCalledWith("p", { name: null, internal_url: null });
  });

  it("a_refused_url_change_shows_on_the_repo_row_and_keeps_the_form", async () => {
    const refusal = "rest already has a URL; remove the repo and add it again to change it";
    await products([alpha({ repos: [repo("rest", "/src/rest", null)] })], {
      upsertRepo: vi.fn().mockRejectedValue(new FleetError(refusal)),
    });
    await userEvent.type(field("Clone URL 1"), "x");
    await click("Save");
    expect(await screen.findByText(refusal)).toBeInTheDocument();
    expect(field("Clone URL 1")).toHaveValue("x");
  });

  it("a_refused_repo_removal_comes_back_with_its_reason", async () => {
    await products([alpha({ repos: [repo("core", "/src/core", null)] })], {
      deleteRepo: vi.fn().mockRejectedValue(new FleetError("core is used by a running task")),
    });
    await click("Remove repo 1");
    expect(screen.queryByRole("textbox", { name: "Repo name 1" })).toBeNull();
    await click("Save");
    expect(await screen.findByText("core is used by a running task")).toBeInTheDocument();
    expect(field("Repo name 1")).toHaveValue("core");
    expect(button("Save")).toBeDisabled();
  });

  it("a_stored_url_is_read_only", async () => {
    await products([alpha({ repos: [repo("rest", null, "https://git.example/rest.git")] })]);
    await userEvent.type(field("Clone URL 1"), "x");
    expect(field("Clone URL 1")).toHaveValue("https://git.example/rest.git");
    expect(button("Save")).toBeDisabled();
  });

  it("no_products_says_so", async () => {
    await management("Products", { products: [] });
    expect(await screen.findByText("No product yet. Create one with New product.")).toBeInTheDocument();
  });

  it("the_tab_new_product_opens_the_dialog", async () => {
    await products([alpha()]);
    await click("+ New product");
    expect(screen.getByRole("dialog", { name: "New product" })).toBeInTheDocument();
  });
});

describe("Products: delete and the list", () => {
  it("delete_used_product_shows_reason", async () => {
    const refusal = "Alpha has 2 tasks; archive or delete them first";
    const client = await products([alpha({ tasks: 2 })], { deleteProduct: vi.fn().mockRejectedValue(new FleetError(refusal)) });
    await click("Delete product");
    expect(screen.getByText("Delete Alpha? This cannot be undone.")).toBeInTheDocument();
    expect(client.deleteProduct).not.toHaveBeenCalled();
    await click("Confirm delete");
    expect(client.deleteProduct).toHaveBeenCalledWith("p");
    expect(await screen.findByText(refusal)).toBeInTheDocument();
    expect(button("Delete product")).toBeInTheDocument();
  });

  it("switching_products_starts_from_the_daemon_copy", async () => {
    await products([alpha(), product("b", "Beta")]);
    await userEvent.type(field("Name"), "!");
    await userEvent.click(screen.getByRole("button", { name: "Beta" }));
    expect(field("Name")).toHaveValue("Beta");
    await userEvent.click(screen.getByRole("button", { name: "Alpha" }));
    expect(field("Name")).toHaveValue("Alpha");
    expect(button("Save")).toBeDisabled();
  });

  it("a_saved_form_waits_for_the_list_then_follows_it", async () => {
    const reloaded = pending<{ products: Product[] }>();
    const list = vi
      .fn()
      .mockResolvedValueOnce({ products: [alpha()] })
      .mockReturnValueOnce(reloaded.promise);
    await products([alpha()], { products: list });
    await userEvent.type(field("Name"), "!");
    await click("Save");
    // Saved, but the list is not back yet: Save waits.
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    expect(button("Save")).toBeDisabled();
    await act(async () => reloaded.resolve({ products: [alpha({ name: "Alpha!" })] }));
    expect(field("Name")).toHaveValue("Alpha!");
    expect(button("Save")).toBeDisabled();
    expect(button("Revert")).toBeDisabled();
  });

  it("deleting_the_active_product_clears_it", async () => {
    localStorage.setItem("fleet.product", "p");
    const list = vi
      .fn()
      .mockResolvedValueOnce({ products: [alpha(), product("b", "Beta")] })
      .mockResolvedValue({ products: [product("b", "Beta")] });
    await products([alpha(), product("b", "Beta")], { products: list });
    const drawer = screen.getByRole("navigation", { name: "Drawer" });
    expect(within(drawer).getByRole("button", { name: "Active product" })).toHaveTextContent("Alpha");
    await click("Delete product");
    await click("Confirm delete");
    await waitFor(() => expect(field("Name")).toHaveValue("Beta"));
    expect(within(drawer).getByRole("button", { name: "Active product" })).toHaveTextContent("Pick a product");
    // egui's `set_active_product(None)`: ids are name slugs, so a product made later under the same name stays inactive.
    expect(localStorage.getItem("fleet.product")).toBeNull();
  });

  it("deleting_another_product_keeps_the_active_one", async () => {
    localStorage.setItem("fleet.product", "b");
    const list = vi
      .fn()
      .mockResolvedValueOnce({ products: [alpha(), product("b", "Beta")] })
      .mockResolvedValue({ products: [product("b", "Beta")] });
    await products([alpha(), product("b", "Beta")], { products: list });
    await click("Delete product");
    await click("Confirm delete");
    await waitFor(() => expect(field("Name")).toHaveValue("Beta"));
    expect(localStorage.getItem("fleet.product")).toBe("b");
  });
});

/** A fresh app on Products, for a test that draws several. */
async function renderProducts(list: Product[]) {
  const { cleanup } = await import("@testing-library/react");
  await products(list);
  return { unmount: cleanup };
}

const combo = (name: string) => screen.getByRole("combobox", { name });
const pick = (name: string, choice: string) => userEvent.selectOptions(combo(name), choice);
const options = (name: string) =>
  within(combo(name))
    .getAllByRole("option")
    .map((o) => o.textContent);
const override = (section: string, key: string, value: string, error: string | null = null): ProductOverride => ({
  section,
  key,
  value,
  error,
});

/** egui's `overrides_harness`: Alpha with these overrides, and a model named `rust` among the definitions. */
async function overrides(list: ProductOverride[], over = {}) {
  const client = await products([alpha({ overrides: list })], {
    definitions: vi.fn().mockResolvedValue([def("orchestrator", "bug-fix"), def("specialist", "rust")]),
    ...over,
  });
  await screen.findByRole("button", { name: "rust" });
  return client;
}

describe("Products: hidden specialists and overrides", () => {
  it("products_tab_saves_hidden_specialists", async () => {
    const stored = alpha({ repos: [repo("core", "/src/core", null)], hidden_specialists: ["stale"] });
    const saved = { ...stored, hidden_specialists: ["reviewer"] };
    const client = await products([stored], {
      definitions: vi
        .fn()
        .mockResolvedValue([
          def("specialist", "reviewer"),
          def("specialist", "rust"),
          def("specialist", "stale", "specialists/stale.md: bad"),
        ]),
      products: vi
        .fn()
        .mockResolvedValueOnce({ products: [stored] })
        .mockResolvedValue({ products: [saved] }),
      putHiddenSpecialists: vi.fn().mockRejectedValueOnce(new FleetError("no specialist named `x`")).mockResolvedValue(saved),
    });
    await screen.findByRole("button", { name: "reviewer" });
    // A hidden one that no longer loads is named, not offered, and not sent.
    expect(screen.getByTitle("This specialist no longer loads: see Fleet, Specialists")).toHaveTextContent("stale");
    expect(screen.queryByRole("button", { name: "stale" })).toBeNull();
    await click("reviewer");
    await click("Save hidden specialists");
    expect(client.putHiddenSpecialists).toHaveBeenLastCalledWith("p", ["reviewer"]);
    expect(await screen.findByText("no specialist named `x`")).toBeInTheDocument();
    // Saved: the draft takes the stored list, so there is nothing left to save.
    await click("Save hidden specialists");
    await waitFor(() => expect(screen.queryByText("no specialist named `x`")).toBeNull());
    expect(button("Save hidden specialists")).toBeDisabled();
  });

  it("overrides_round_trip_through_the_client", async () => {
    const client = await overrides([]);
    expect(button("Save overrides")).toBeDisabled();
    await click("Add override");
    await pick("Override section 1", "limits");
    await pick("Override key 1", "max_steps");
    await userEvent.type(field("Override value 1"), "30");
    await click("Save overrides");
    expect(client.putOverrides).toHaveBeenLastCalledWith("p", [{ section: "limits", key: "max_steps", value: "30" }]);
  });

  it("a_model_override_offers_the_definitions", async () => {
    const client = await overrides([]);
    await click("Add override");
    expect(options("Override key 1")).toEqual(["key", "bug-fix", "rust"]);
    await pick("Override key 1", "rust");
    await userEvent.type(field("Override value 1"), "opus");
    await click("Save overrides");
    expect(client.putOverrides).toHaveBeenLastCalledWith("p", [{ section: "models", key: "rust", value: "opus" }]);
  });

  it("removing_the_last_override_saves_an_empty_list", async () => {
    const client = await overrides([override("limits", "max_steps", "30")]);
    await click("Remove override 1");
    await click("Save overrides");
    expect(client.putOverrides).toHaveBeenLastCalledWith("p", []);
  });

  it("refused_overrides_show_the_message", async () => {
    const why = "limits.max_steps must be a positive whole number";
    await overrides([], { putOverrides: vi.fn().mockRejectedValue(new FleetError(why)) });
    await click("Add override");
    await pick("Override section 1", "limits");
    await pick("Override key 1", "max_steps");
    await userEvent.type(field("Override value 1"), "0");
    await click("Save overrides");
    expect(await screen.findByText(why)).toBeInTheDocument();
    // The edits are kept, and can be sent again.
    expect(field("Override value 1")).toHaveValue("0");
    expect(button("Save overrides")).toBeEnabled();
  });

  it("saved_overrides_in_any_order_leave_nothing_to_save", async () => {
    // The daemon answers sorted by section and key, and trimmed.
    const stored = alpha({ overrides: [override("limits", "max_steps", "30"), override("models", "rust", "opus")] });
    await overrides([], {
      products: vi
        .fn()
        .mockResolvedValueOnce({ products: [alpha()] })
        .mockResolvedValue({ products: [stored] }),
      putOverrides: vi.fn().mockResolvedValue(stored),
    });
    await click("Add override");
    await pick("Override key 1", "rust");
    await userEvent.type(field("Override value 1"), " opus ");
    await click("Add override");
    await pick("Override section 2", "limits");
    await pick("Override key 2", "max_steps");
    await userEvent.type(field("Override value 2"), "30");
    await click("Save overrides");
    // The saved rows are the edited rows.
    await waitFor(() => expect(field("Override value 1")).toHaveValue("30"));
    expect(button("Save overrides")).toBeDisabled();
  });

  it("a_clean_override_list_follows_the_daemon", async () => {
    const moved = alpha({ name: "Alpha!", overrides: [override("limits", "max_steps", "50")] });
    await overrides([override("limits", "max_steps", "30")], {
      products: vi
        .fn()
        .mockResolvedValueOnce({ products: [alpha({ overrides: [override("limits", "max_steps", "30")] })] })
        .mockResolvedValue({ products: [moved] }),
    });
    // Any answer of the list will do; a save of the name asks for one.
    await userEvent.type(field("Name"), "!");
    await click("Save");
    await waitFor(() => expect(field("Override value 1")).toHaveValue("50"));
    expect(button("Save overrides")).toBeDisabled();
  });

  it("invalid_override_shows_its_error", async () => {
    await overrides([override("models", "ghost", "opus", "no definition named ghost")]);
    expect(screen.getByText("no definition named ghost")).toBeInTheDocument();
    expect(field("Override value 1")).toHaveValue("opus");
    // A key the definitions no longer list is shown as it is.
    expect(combo("Override key 1")).toHaveValue("ghost");
  });

  it("an_override_shows_the_value_it_replaces_and_where_that_came_from", async () => {
    await overrides([override("limits", "max_steps", "30"), override("models", "rust", "opus")], {
      effectiveSettings: vi.fn().mockResolvedValue({
        values: [
          { section: "limits", key: "max_steps", value: "20", source: "default" },
          { section: "models", key: "rust", value: null, source: "default" },
        ],
      }),
    });
    expect(await screen.findByText("replaces 20 (default)")).toBeInTheDocument();
    expect(screen.getByText("replaces persona default")).toBeInTheDocument();
  });

  it("concurrent_tasks_override_replaces_no_cap", async () => {
    await overrides([], {
      effectiveSettings: vi.fn().mockResolvedValue({
        values: [{ section: "limits", key: "max_concurrent_tasks", value: null, source: "default" }],
      }),
    });
    await click("Add override");
    await pick("Override section 1", "limits");
    expect(options("Override key 1")).not.toContain("max_active_products");
    await pick("Override key 1", "max_concurrent_tasks");
    expect(await screen.findByText("replaces no cap")).toBeInTheDocument();
  });

  it("a_pr_style_override_offers_the_styles", async () => {
    const styles = ["conventional-semver-pre1.0", "conventional-semver", "conventional", "minimal", "plain"];
    const client = await products([alpha()], {
      definitions: vi
        .fn()
        .mockResolvedValue([...styles.map((n) => def("pr_style", n)), def("pr_style", "broken", "pr_styles/broken.md: no title")]),
    });
    await click("Add override");
    await pick("Override section 1", "pr");
    await pick("Override key 1", "style");
    await waitFor(() => expect(options("Override value 1")).toEqual(["value", ...styles]));
    await pick("Override value 1", "conventional");
    await click("Save overrides");
    expect(client.putOverrides).toHaveBeenLastCalledWith("p", [{ section: "pr", key: "style", value: "conventional" }]);
  });

  it("mode_and_effort_overrides_offer_only_the_allowed_values", async () => {
    const client = await overrides([]);
    await click("Add override");
    await pick("Override section 1", "modes");
    await pick("Override key 1", "rust");
    expect(options("Override value 1")).toEqual(["value", "auto", "acceptEdits", "manual", "dontAsk"]);
    await pick("Override value 1", "manual");
    await click("Add override");
    await pick("Override section 2", "efforts");
    await pick("Override key 2", "rust");
    await pick("Override value 2", "xhigh");
    await click("Save overrides");
    expect(client.putOverrides).toHaveBeenLastCalledWith("p", [
      { section: "modes", key: "rust", value: "manual" },
      { section: "efforts", key: "rust", value: "xhigh" },
    ]);
  });
});
