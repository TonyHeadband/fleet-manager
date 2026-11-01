import { useCallback, useEffect, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { EffectiveSettings } from "../api/generated/EffectiveSettings";
import type { Product } from "../api/generated/Product";
import type { Load } from "../shell/choices";
import { useInternal } from "./internal";
import { drafts, editRow, newOverride, sameRows, toOverride, type OverrideDraft } from "./overrides";
import { diff, emptyForm, formOf, type ProductForm, type ProductSave } from "./productForm";

// A port of crates/fleet-gui/src/views/management/products.rs's ProductsState and of the product half of
// app/products.rs: the selected product, its form, and the calls a save or a delete makes.

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
  /** The hidden names no persona and no built-in defines any more, as the daemon reports them. */
  staleHidden: string[];
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
  staleHidden: [],
  hiddenBusy: false,
  hiddenError: null,
  overrides: [],
  savedOverrides: [],
  overridesBusy: false,
  overridesError: null,
};

/** Order-insensitive. */
export const sameNames = (a: string[], b: string[]) => a.length === b.length && a.every((n) => b.includes(n));

export const overridesDirty = (st: ProductsState) => !sameRows(st.overrides, st.savedOverrides);

/**
 * What Save hidden specialists sends: every hidden name some persona or built-in still defines, not only the ones the
 * drawer's persona has.
 */
export const hiddenToSend = (st: ProductsState) => st.hidden.filter((n) => !st.staleHidden.includes(n));

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
      staleHidden: p.stale_hidden_specialists,
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
    staleHidden: p.stale_hidden_specialists,
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

/**
 * egui's `hidden_done` and `overrides_done`: on success both lists take the stored ones (the daemon sorts and trims
 * overrides, so the edited order would otherwise stay dirty); a refusal keeps the draft and shows the reason.
 */
export function hiddenDone(st: ProductsState, id: string, result: Product | string): ProductsState {
  if (st.forProduct !== id) return st;
  if (typeof result === "string") return { ...st, hiddenBusy: false, hiddenError: result };
  const hidden = result.hidden_specialists;
  return { ...st, hiddenBusy: false, hidden, savedHidden: hidden, staleHidden: result.stale_hidden_specialists, hiddenError: null };
}

export function overridesDone(st: ProductsState, id: string, result: Product | string): ProductsState {
  if (st.forProduct !== id) return st;
  if (typeof result === "string") return { ...st, overridesBusy: false, overridesError: result };
  return { ...st, overridesBusy: false, overrides: drafts(result), savedOverrides: drafts(result), overridesError: null };
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
  /** A product was deleted: App lets go of it if it was the active one. */
  deleted: (id: string) => void,
  /** The drawer's active persona, whose definitions the settings are read for. */
  persona: string | null,
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
  // Kept with the tab, as egui's InternalState in ProductsState; it follows the product the form is for.
  const internal = useInternal(client, st.forProduct);
  // The settings without any product's overrides, which each override row is set against; asked after each list.
  const [effective, setEffective] = useState<EffectiveSettings | null>(null);

  // A list answered: a save no longer waits for it, and the form follows the daemon's copy.
  useEffect(() => {
    if (products.state !== "loading") update((s) => ({ ...s, busy: false }));
  }, [products, update]);
  useEffect(() => {
    if (shown && products.state === "ready") update((s) => sync(s, products.value));
  }, [shown, products, update]);
  // The old persona's settings are not the new one's: dropped now, and its late answer ignored below.
  useEffect(() => setEffective(null), [persona]);
  useEffect(() => {
    if (!shown || products.state !== "ready" || persona === null) return;
    let stale = false;
    client.effectiveSettings(persona).then(
      (e) => !stale && setEffective(e),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [client, shown, products, persona]);

  /** One of the product's own lists is sent; `done` takes the answer, then the list is asked for on success. */
  const send = async (busy: "hiddenBusy" | "overridesBusy", put: (id: string) => Promise<Product>, done: typeof hiddenDone) => {
    const id = now.current.forProduct;
    if (id === null || now.current[busy]) return;
    update((x) => ({ ...x, [busy]: true, [busy === "hiddenBusy" ? "hiddenError" : "overridesError"]: null }));
    const result = await put(id).catch((e: unknown) => message(e));
    update((x) => done(x, id, result));
    if (typeof result !== "string") reload();
  };

  return {
    st,
    effective,
    internal,
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
    toggleHidden: (name: string) =>
      update((s) => ({ ...s, hidden: s.hidden.includes(name) ? s.hidden.filter((n) => n !== name) : [...s.hidden, name] })),
    saveHidden: (names: string[]) => send("hiddenBusy", (id) => client.putHiddenSpecialists(id, names), hiddenDone),
    addOverride: () => update((s) => ({ ...s, overrides: [...s.overrides, newOverride()] })),
    removeOverride: (n: number) => update((s) => ({ ...s, overrides: s.overrides.filter((_, i) => i !== n) })),
    editOverride: (n: number, change: Parameters<typeof editRow>[1]) =>
      update((s) => ({ ...s, overrides: s.overrides.map((r, i) => (i === n ? editRow(r, change) : r)) })),
    saveOverrides: () => send("overridesBusy", (id) => client.putOverrides(id, now.current.overrides.map(toOverride)), overridesDone),
    askDelete: () => update((s) => ({ ...s, confirmDelete: true, deleteError: null })),
    cancelDelete: () => update((s) => ({ ...s, confirmDelete: false })),
    confirmDelete: (id: string) =>
      client.deleteProduct(id).then(
        () => {
          deleted(id);
          reload();
        },
        (e: unknown) => update((s) => (s.forProduct === id ? { ...s, confirmDelete: false, deleteError: message(e) } : s)),
      ),
  };
}

export type ProductsApi = ReturnType<typeof useProductsTab>;
