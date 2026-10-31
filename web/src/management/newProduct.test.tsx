import { readFileSync } from "node:fs";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import { product } from "../test/fixtures";
import { click, managedPersona, management, pending } from "./harness";

// Ports of egui's New product tests in crates/fleet-gui/tests/app.rs, and the dialog's own rules.

const portal = product("p", "Portal");
const core = product("core", "Core");

/** The app with Portal listed; once Core is made, the daemon lists it too. */
async function app(over = {}) {
  const products = vi
    .fn()
    .mockResolvedValueOnce({ products: [portal] })
    .mockResolvedValue({ products: [portal, core] });
  const client = await management("Overview", {
    products: [portal],
    personas: [managedPersona("a", "Work", true), managedPersona("b", "Home", true)],
    over: { products, ...over },
  });
  await click("New product");
  return client;
}

const dialog = () => screen.getByRole("dialog", { name: "New product" });
const field = (name: string) => within(dialog()).getByRole("textbox", { name });
const save = () => within(dialog()).getByRole("button", { name: "Save" });

beforeEach(() => localStorage.clear());

describe("New product", () => {
  it("new_product_creates_and_selects", async () => {
    const client = await app();
    await userEvent.type(field("Name"), "Core");
    await userEvent.click(save());
    expect(client.createProduct).toHaveBeenCalledWith({ name: "Core", default_persona: null, internal_url: null });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const drawer = screen.getByRole("navigation", { name: "Drawer" });
    await waitFor(() => expect(within(drawer).getByRole("button", { name: "Active product" })).toHaveTextContent("Core"));
    expect(localStorage.getItem("fleet.product")).toBe("core");
  });

  it("new_product_sends_the_internal_url_when_typed", async () => {
    const client = await app();
    await userEvent.type(field("Name"), "Core");
    await userEvent.type(field("Internal repo URL"), "https://git.example/notes.git");
    await userEvent.click(save());
    expect(client.createProduct).toHaveBeenCalledWith({
      name: "Core",
      default_persona: null,
      internal_url: "https://git.example/notes.git",
    });
  });

  it("a_refused_create_shows_under_the_form", async () => {
    await app({ createProduct: vi.fn().mockRejectedValue(new FleetError("a product named Core exists")) });
    await userEvent.type(field("Name"), "Core");
    await userEvent.click(save());
    expect(await within(dialog()).findByText("a product named Core exists")).toBeInTheDocument();
    expect(field("Name")).toHaveValue("Core");
  });

  it("a_repo_the_new_product_could_not_add_is_a_message", async () => {
    const client = await app({ upsertRepo: vi.fn().mockRejectedValue(new FleetError("no such folder")) });
    await userEvent.type(field("Name"), "Core");
    await userEvent.click(within(dialog()).getByRole("button", { name: "Add repo" }));
    await userEvent.type(field("Repo name 1"), "docs");
    await userEvent.type(field("Local path 1"), "/src/docs");
    await userEvent.click(save());
    expect(await screen.findByText("Created Core, but could not add the repo docs: no such folder")).toBeInTheDocument();
    expect(client.upsertRepo).toHaveBeenCalledWith("core", { name: "docs", path: "/src/docs", url: null });
    // The product was made all the same.
    await waitFor(() => expect(localStorage.getItem("fleet.product")).toBe("core"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("save_waits_for_a_name_and_valid_repos", async () => {
    await app();
    expect(save()).toBeDisabled();
    await userEvent.type(field("Name"), "Core");
    expect(save()).toBeEnabled();
    await userEvent.click(within(dialog()).getByRole("button", { name: "Add repo" }));
    await userEvent.type(field("Repo name 1"), "docs");
    expect(save()).toBeDisabled();
  });

  it("cancel_and_escape_close_it_unsaved", async () => {
    const client = await app();
    await userEvent.click(within(dialog()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await click("New product");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(client.createProduct).not.toHaveBeenCalled();
  });

  it("a_refused_create_is_in_the_danger_tone", () => {
    const css = readFileSync("src/management/management.css", "utf8");
    expect(css).toMatch(/\.dialog\.new-product \.danger \{[^}]*color: var\(--danger\)/);
  });

  it("a_create_refused_after_the_dialog_closed_is_a_message", async () => {
    const made = pending<never>();
    await app({ createProduct: vi.fn().mockReturnValue(made.promise) });
    await userEvent.type(field("Name"), "Core");
    await userEvent.click(save());
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => made.reject(new FleetError("a product named Core exists")));
    expect(await screen.findByText("Could not create the product: a product named Core exists")).toBeInTheDocument();
  });
});
