import { useCallback, useEffect, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { Product } from "../api/generated/Product";
import type { Load } from "../shell/choices";
import { diff, emptyForm, formOf, type ProductForm, type ProductSave } from "./productForm";

// A port of crates/fleet-gui/src/views/management/products.rs's ProductsState and of the product half of
// app/products.rs: the selected product, its form, and the calls a save or a delete makes.

/** An override row as being edited; `error` is the daemon's, from a read. */
export interface OverrideDraft {
  section: string;
  key: string;
  value: string;
  error: string | null;
}

export interface ProductsState {
  selected: string | null;
  form: ProductForm;
  /** The product as the daemon has it, as a form: what `form` is compared with. */
  saved: ProductForm;
  /** The product `form` was made for. */
  forProduct: string | null;
  /** A save is in flight, or its answer is awaiting the refreshed list. */
  busy: boolean;
  /** The daemon's reason for refusing the product's own fields. */
  error: string | null;
  /** By repo name. */
  repoErrors: Record<string, string>;
  /** Delete product was clicked once and waits for Confirm delete. */
  confirmDelete: boolean;
  deleteError: string | null;
  /** The hidden specialists as being edited, and as the daemon has them. */
  hidden: string[];
  savedHidden: string[];
  hiddenBusy: boolean;
  hiddenError: string | null;
  /** The overrides as being edited, and as the daemon has them. */
  overrides: OverrideDraft[];
  savedOverrides: OverrideDraft[];
  overridesBusy: boolean;
  overridesError: string | null;
}

const initial: ProductsState = {
  selected: null,
  form: emptyForm,
  saved: emptyForm,
  forProduct: null,
  busy: false,
  error: null,
  repoErrors: {},
  confirmDelete: false,
  deleteError: null,
  hidden: [],
  savedHidden: [],
  hiddenBusy: false,
  hiddenError: null,
  overrides: [],
  savedOverrides: [],
  overridesBusy: false,
  overridesError: null,
};

export const drafts = (p: Product): OverrideDraft[] =>
  p.overrides.map((o) => ({ section: o.section, key: o.key, value: o.value, error: o.error ?? null }));

/** Order-sensitive, on what is sent: a row's error is not part of it. */
export const sameRows = (a: OverrideDraft[], b: OverrideDraft[]) =>
  a.length === b.length && a.every((r, i) => r.section === b[i]!.section && r.key === b[i]!.key && r.value === b[i]!.value);

/** Order-insensitive. */
export const sameNames = (a: string[], b: string[]) => a.length === b.length && a.every((n) => b.includes(n));

const isEmpty = (save: ProductSave) => save.update === null && save.upserts.length === 0 && save.removes.length === 0;

export const dirty = (st: ProductsState) => st.forProduct !== null && !isEmpty(diff(st.forProduct, st.form, st.saved));

/**
 * egui's `sync`: the selection stays on a listed product and the form on that product. Another product starts from
 * the daemon's copy; for the same one, edits are kept when the daemon's copy changes and a clean form follows it.
 */
export function sync(st: ProductsState, products: Product[]): ProductsState {
  const selected = products.some((p) => p.id === st.selected) ? st.selected : (products[0]?.id ?? null);
  const p = products.find((l) => l.id === selected);
  if (!p) return { ...st, selected, forProduct: null };
  const fresh = formOf(p);
  if (st.forProduct !== p.id) {
    return {
      ...initial,
      selected,
      form: fresh,
      saved: fresh,
      forProduct: p.id,
      hidden: p.hidden_specialists,
      savedHidden: p.hidden_specialists,
      overrides: drafts(p),
      savedOverrides: drafts(p),
    };
  }
  const changed = JSON.stringify(st.saved) !== JSON.stringify(fresh);
  return {
    ...st,
    selected,
    form: changed && !dirty(st) ? fresh : st.form,
    saved: fresh,
    overrides: sameRows(st.overrides, st.savedOverrides) ? drafts(p) : st.overrides,
    savedOverrides: drafts(p),
    hidden: sameNames(st.hidden, st.savedHidden) ? p.hidden_specialists : st.hidden,
    savedHidden: p.hidden_specialists,
  };
}

/** A save's answer: the product's own refusal, and the repos refused, by name. */
export interface ProductEdit {
  id: string;
  productError: string | null;
  repoErrors: [string, string][];
}

/** egui's `edit_done`. Whatever succeeded is in the list that is about to be fetched. */
export function editDone(st: ProductsState, e: ProductEdit): ProductsState {
  if (st.forProduct !== e.id) return st;
  const failed = e.productError !== null || e.repoErrors.length > 0;
  const repoErrors = Object.fromEntries(e.repoErrors);
  // A removal the daemon refused: the row comes back, carrying the reason.
  const back = st.saved.repos.filter((s) => s.name in repoErrors && !st.form.repos.some((r) => r.name.trim() === s.name));
  return {
    ...st,
    error: e.productError,
    repoErrors,
    form: back.length ? { ...st.form, repos: [...st.form.repos, ...back] } : st.form,
    busy: failed ? false : st.busy,
  };
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The tab's state and calls, kept above Management so a form being edited outlives the tab, as egui's does. Each
 * answer of the products list, a failed one too, is egui's `refreshed`.
 */
export function useProductsTab(
  client: FleetClient,
  products: Load<Product[]>,
  reload: () => void,
  reloadRepos: () => void,
  /** The tab is drawn: egui syncs the form only then. */
  shown: boolean,
) {
  const base = products.state === "ready" ? products.value : null;
  const [st, setSt] = useState<ProductsState>(initial);
  const now = useRef(st);
  now.current = st;
  const update = useCallback((f: (s: ProductsState) => ProductsState) => {
    now.current = f(now.current);
    setSt(now.current);
  }, []);
  const list = useRef(base);
  list.current = base;

  // A list answered: a save no longer waits for it, and the form follows the daemon's copy.
  useEffect(() => {
    if (products.state !== "loading") update((s) => ({ ...s, busy: false }));
  }, [products, update]);
  useEffect(() => {
    if (shown && products.state === "ready") update((s) => sync(s, products.value));
  }, [shown, products, update]);

  return {
    st,
    update,
    select: (id: string) => list.current && update((s) => sync({ ...s, selected: id }, list.current!)),
    setForm: (form: ProductForm) => update((s) => ({ ...s, form })),
    revert: () => update((s) => ({ ...s, form: s.saved, error: null, repoErrors: {} })),
    /** Sends the product's fields, then each repo change, each on its own; a refusal is kept by what it refused. */
    save: async () => {
      const s = now.current;
      if (s.forProduct === null || s.busy) return;
      const save = diff(s.forProduct, s.form, s.saved);
      update((x) => ({ ...x, busy: true, error: null, repoErrors: {} }));
      const edit: ProductEdit = { id: save.id, productError: null, repoErrors: [] };
      if (save.update) await client.updateProduct(save.id, save.update).catch((e: unknown) => (edit.productError = message(e)));
      for (const r of save.upserts) await client.upsertRepo(save.id, r).catch((e: unknown) => edit.repoErrors.push([r.name, message(e)]));
      for (const name of save.removes)
        await client.deleteRepo(save.id, name).catch((e: unknown) => edit.repoErrors.push([name, message(e)]));
      update((x) => editDone(x, edit));
      reload();
      reloadRepos();
    },
    askDelete: () => update((s) => ({ ...s, confirmDelete: true, deleteError: null })),
    cancelDelete: () => update((s) => ({ ...s, confirmDelete: false })),
    confirmDelete: (id: string) =>
      client
        .deleteProduct(id)
        .then(reload, (e: unknown) => update((s) => (s.forProduct === id ? { ...s, confirmDelete: false, deleteError: message(e) } : s))),
  };
}

export type ProductsApi = ReturnType<typeof useProductsTab>;
