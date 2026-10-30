import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { Product } from "../api/generated/Product";
import { click, managedProduct as product, managedRepo as repo, management, pending } from "./harness";

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
    expect(within(drawer).getByRole("combobox", { name: "Active product" })).toHaveValue("p");
    await click("Delete product");
    await click("Confirm delete");
    await waitFor(() => expect(field("Name")).toHaveValue("Beta"));
    expect(within(drawer).getByRole("combobox", { name: "Active product" })).not.toHaveValue("p");
  });
});

/** A fresh app on Products, for a test that draws several. */
async function renderProducts(list: Product[]) {
  const { cleanup } = await import("@testing-library/react");
  await products(list);
  return { unmount: cleanup };
}
