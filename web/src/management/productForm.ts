import type { NewProduct } from "../api/generated/NewProduct";
import type { Product } from "../api/generated/Product";
import type { ProductUpdate } from "../api/generated/ProductUpdate";
import type { RepoUpsert } from "../api/generated/RepoUpsert";

// A port of crates/fleet-gui/src/components/product_form.rs: the one form for a product, New product's and the
// Products tab's, and what saving an edited one sends.

/** A repo row being typed. A row with nothing in it is ignored; a named one needs a path or a URL. */
export interface RepoDraft {
  name: string;
  path: string;
  url: string;
  /** The product already has this repo: its name is fixed (a rename is Remove and Add). */
  existing: boolean;
  /** The stored repo has a URL: it cannot be changed or cleared (Remove the repo and add it again). */
  hadUrl: boolean;
  /** The stored repo has a path: it can be changed, but not cleared. */
  hadPath: boolean;
}

export interface ProductForm {
  name: string;
  /** A persona id; null is "No default". */
  defaultPersona: string | null;
  /** The internal repo's URL; empty is none. */
  internalUrl: string;
  repos: RepoDraft[];
}

/** What saving an edited product sends: the product's own fields, then the repos that changed. */
export interface ProductSave {
  id: string;
  update: Partial<ProductUpdate> | null;
  upserts: RepoUpsert[];
  removes: string[];
}

export const newRepo = (): RepoDraft => ({ name: "", path: "", url: "", existing: false, hadUrl: false, hadPath: false });

export const emptyForm: ProductForm = { name: "", defaultPersona: null, internalUrl: "", repos: [] };

export const blank = (r: RepoDraft) => [r.name, r.path, r.url].every((s) => s.trim() === "");

export const valid = (r: RepoDraft) => r.name.trim() !== "" && !(r.path.trim() === "" && r.url.trim() === "");

const some = (s: string) => (s.trim() === "" ? null : s.trim());

const upsert = (r: RepoDraft): RepoUpsert => ({ name: r.name.trim(), path: some(r.path), url: some(r.url) });

export function formOf(p: Product): ProductForm {
  return {
    name: p.name,
    defaultPersona: p.default_persona,
    internalUrl: p.internal_url ?? "",
    repos: p.repos.map((r) => ({
      name: r.name,
      path: r.path ?? "",
      url: r.url ?? "",
      existing: true,
      hadUrl: r.url !== null,
      hadPath: r.path !== null,
    })),
  };
}

/**
 * What differs from `saved` (the product as the daemon has it). Emptying a path or URL of a stored repo is not a
 * change: the daemon only fills them in.
 */
export function diff(id: string, form: ProductForm, saved: ProductForm): ProductSave {
  const update: Partial<ProductUpdate> = {};
  const name = form.name.trim();
  if (name !== "" && name !== saved.name) update.name = name;
  if (form.defaultPersona !== saved.defaultPersona) update.default_persona = form.defaultPersona;
  const url = form.internalUrl.trim();
  if (url !== saved.internalUrl) update.internal_url = some(url);
  const upserts: RepoUpsert[] = [];
  for (const r of form.repos.filter((r) => !blank(r))) {
    const stored = saved.repos.find((s) => s.name === r.name.trim());
    const up = upsert(r);
    if (!stored) {
      upserts.push(up);
      continue;
    }
    const changed = { ...up, path: up.path !== stored.path ? up.path : null, url: up.url !== stored.url ? up.url : null };
    if (changed.path !== null || changed.url !== null) upserts.push(changed);
  }
  const removes = saved.repos.filter((s) => !form.repos.some((r) => r.name.trim() === s.name)).map((s) => s.name);
  const changed = Object.keys(update).length > 0;
  // `name` is always in the body: null leaves it alone, as egui's ProductUpdate serialises.
  return { id, update: changed ? { name: null, ...update } : null, upserts, removes };
}

/** Saving needs a name, and every row that has anything in it a name and a path or URL. */
export const canSave = (form: ProductForm) => form.name.trim() !== "" && form.repos.every((r) => blank(r) || valid(r));

export const newProduct = (form: ProductForm): NewProduct => ({
  name: form.name.trim(),
  default_persona: form.defaultPersona,
  internal_url: some(form.internalUrl),
});

/** The rows to send, without the blank ones. */
export const repoUpserts = (form: ProductForm) => form.repos.filter((r) => !blank(r)).map(upsert);
