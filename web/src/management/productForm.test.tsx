import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { Product } from "../api/generated/Product";
import type { RepoInfo } from "../api/generated/RepoInfo";
import { optionsOf } from "../test/fixtures";
import { managedPersona } from "./harness";
import { ProductFields } from "./ProductFields";
import { canSave, diff, formOf, newProduct, repoUpserts, type ProductForm } from "./productForm";

// egui's product_form.rs: the form New product and Management → Products both draw, and what a save sends.

const repo = (name: string, path: string | null, url: string | null): RepoInfo => ({
  product: "p",
  name,
  path,
  url,
  workflow: { state: "missing", base: "main" },
  env: [],
});

const alpha = (over: Partial<Product> = {}): Product =>
  ({
    id: "p",
    name: "Alpha",
    default_persona: null,
    internal_url: null,
    repos: [repo("core", "/src/core", null), repo("rest", null, "https://git.example/rest.git")],
    releases: [],
    tasks: 0,
    overrides: [],
    hidden_specialists: [],
    stale_hidden_specialists: [],
    ...over,
  }) as Product;

describe("the product form's save", () => {
  it("sends_only_what_changed", () => {
    const saved = formOf(alpha());
    const form: ProductForm = {
      ...saved,
      name: "Alpha! ",
      repos: [
        { ...saved.repos[0]!, url: "https://git.example/core.git" },
        { name: "docs", path: "/src/docs", url: "", existing: false, hadUrl: false, hadPath: false },
      ],
    };
    // As egui's products_tab_edits_repo_url: core gets its URL, docs is added, rest is removed.
    expect(diff("p", form, saved)).toEqual({
      id: "p",
      update: { name: "Alpha!" },
      upserts: [
        { name: "core", path: null, url: "https://git.example/core.git" },
        { name: "docs", path: "/src/docs", url: null },
      ],
      removes: ["rest"],
    });
    expect(diff("p", saved, saved)).toEqual({ id: "p", update: null, upserts: [], removes: [] });
  });

  it("sets_and_clears_the_internal_url", () => {
    const saved = formOf(alpha());
    expect(diff("p", { ...saved, internalUrl: "https://git.example/notes.git" }, saved).update).toEqual({
      name: null,
      internal_url: "https://git.example/notes.git",
    });
    const stored = formOf(alpha({ internal_url: "https://git.example/notes.git" }));
    expect(stored.internalUrl).toBe("https://git.example/notes.git");
    expect(diff("p", { ...stored, internalUrl: "" }, stored).update).toEqual({ name: null, internal_url: null });
    expect(diff("p", { ...stored, name: "Alpha!" }, stored).update).toEqual({ name: "Alpha!" });
  });

  it("a_cleared_stored_path_or_a_blank_row_is_no_change", () => {
    const saved = formOf(alpha());
    const form = { ...saved, repos: [{ ...saved.repos[0]!, path: "" }, saved.repos[1]!, blankRow] };
    expect(diff("p", form, saved)).toEqual({ id: "p", update: null, upserts: [], removes: [] });
  });

  it("saving_needs_a_name_and_whole_rows", () => {
    const saved = formOf(alpha());
    expect(canSave(saved)).toBe(true);
    expect(canSave({ ...saved, name: " " })).toBe(false);
    expect(canSave({ ...saved, repos: [...saved.repos, { ...blankRow, name: "docs" }] })).toBe(false);
    expect(canSave({ ...saved, repos: [...saved.repos, blankRow] })).toBe(true);
  });

  it("a_new_product_and_its_repos", () => {
    const form: ProductForm = {
      name: " Core ",
      defaultPersona: null,
      internalUrl: " ",
      repos: [blankRow, { ...blankRow, name: "web", url: "https://git.example/web.git" }],
    };
    expect(newProduct(form)).toEqual({ name: "Core", default_persona: null, internal_url: null });
    expect(repoUpserts(form)).toEqual([{ name: "web", path: null, url: "https://git.example/web.git" }]);
  });
});

const blankRow = { name: "", path: "", url: "", existing: false, hadUrl: false, hadPath: false };

function Fields({ product, errors = {} }: { product: Product; errors?: Record<string, string> }) {
  const [form, setForm] = useState(formOf(product));
  return (
    <ProductFields
      form={form}
      onChange={setForm}
      personas={[managedPersona("a", "Work", true), managedPersona("d", "Draft", false)]}
      repoInfo={() => undefined}
      repoErrors={errors}
    />
  );
}

describe("the product form's fields", () => {
  it("draws_egui_s_fields_and_rules", async () => {
    render(<Fields product={alpha()} />);
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveAttribute("placeholder", "What the product is called");
    // Only ready personas can be the default.
    expect(await optionsOf("Default persona")).toEqual(["No default", "Work"]);
    expect(screen.getByRole("textbox", { name: "Internal repo URL" })).toHaveAttribute("placeholder", "https://github.com/org/notes.git");
    expect(screen.getByText("HTTPS URL. Cloned with the default persona's GitHub login; tasks push their notes here.")).toBeInTheDocument();
    // A stored repo's name is fixed, and so is a stored URL.
    expect(screen.getByRole("textbox", { name: "Repo name 1" })).toHaveAttribute("readonly");
    const url = screen.getByRole("textbox", { name: "Clone URL 2" });
    expect(url).toHaveAttribute("readonly");
    expect(url).toHaveAttribute("title", "Remove the repo and add it again to change its URL");
    expect(screen.getByRole("textbox", { name: "Local path 1" })).not.toHaveAttribute("readonly");
    await userEvent.clear(screen.getByRole("textbox", { name: "Local path 1" }));
    expect(screen.getByText("A path cannot be cleared; Revert restores it.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Add repo" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Repo name 3" }), "docs");
    expect(screen.getByText("A repo needs a name and a local path or a clone URL.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Remove repo 3" }));
    expect(screen.queryByRole("textbox", { name: "Repo name 3" })).toBeNull();
  });

  it("a_repo_error_shows_under_its_row", () => {
    render(<Fields product={alpha()} errors={{ rest: "rest already has a URL" }} />);
    expect(screen.getByText("rest already has a URL")).toBeInTheDocument();
  });
});
