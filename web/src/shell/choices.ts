import type { Persona } from "../api/generated/Persona";
import type { FleetDisplay } from "../board/groups";
import type { Product } from "../api/generated/Product";
import type { SpawnRequest } from "../api/generated/SpawnRequest";
import type { Native } from "../native/native";

/** A list the daemon is asked for once and retried while it fails. */
/** `error` on a ready load: the last refresh failed, and the value is the one before it. */
export type Load<T> = { state: "loading" } | { state: "failed"; error?: string } | { state: "ready"; value: T; error?: string };

type ChoiceKey = "fleet.persona" | "fleet.product" | "fleet.display";

const KEYS: Record<ChoiceKey, "persona" | "product" | "display"> = {
  "fleet.persona": "persona",
  "fleet.product": "product",
  "fleet.display": "display",
};

type SavedChoices = Awaited<ReturnType<Native["readChoices"]>>;

/** In the app the choices live in egui's `gui.toml`, read once at start and written through the bridge, so both apps
 * share them; a browser keeps its own in `localStorage`. */
let app: { native: Native; saved: SavedChoices } | null = null;

export function setChoiceStore(store: { native: Native; saved: SavedChoices } | null) {
  app = store && { native: store.native, saved: { ...store.saved } };
}

/** The choice saved. Storage can be blocked (private windows, cleared site data): that reads as none. */
export function readChoice(key: ChoiceKey): string | null {
  if (app) return app.saved[KEYS[key]];
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeChoice(key: ChoiceKey, id: string | null): void {
  if (app) {
    if (key === "fleet.display") app.saved.display = id ?? "compact";
    else app.saved[KEYS[key] as "persona" | "product"] = id;
    // Not saved: the choice still holds for this window.
    void app.native.writeChoice(key, id).catch(() => {});
    return;
  }
  try {
    if (id === null) localStorage.removeItem(key);
    else localStorage.setItem(key, id);
  } catch {
    // Not saved: the choice still holds for this page.
  }
}

/** The board's fleet display this browser saved; an unknown or missing one reads as Compact, as Lite's
 * `lenient_fleet_display` does. */
export function readFleetDisplay(): FleetDisplay {
  const saved = readChoice("fleet.display");
  return saved === "show" || saved === "compact" || saved === "hide" ? saved : "compact";
}

/** Mirrors Lite's `FleetApp::active_product`: a saved product counts until a loaded list says it is gone. */
export function activeProduct(products: Load<Product[]>, saved: string | null): string | null {
  if (saved === null) return null;
  return products.state !== "ready" || products.value.some((p) => p.id === saved) ? saved : null;
}

/** Why New task cannot start now, in Lite's words: its persona check, then its product check. */
export function newTaskBlocker(personas: Load<Persona[]>, products: Load<Product[]>, savedProduct: string | null): string | null {
  switch (personas.state) {
    case "loading":
      return "Loading personas…";
    case "failed":
      return "Could not load the personas; retrying";
    case "ready":
      if (!personas.value.some((p) => p.ready)) return "Set up a persona first";
  }
  switch (products.state) {
    case "loading":
      return "Loading products…";
    // A saved choice is enough to start a task; only a missing one needs the list.
    case "failed":
      return savedProduct === null ? "Could not load the products; retrying" : null;
    case "ready":
      if (products.value.length === 0) return "Create a product first";
      return activeProduct(products, savedProduct) === null ? "Pick a product" : null;
  }
}

/** The persona a new task starts under, as Lite's dialog picks it: the product's default, the active one, then the first ready. */
export function spawnPersona(personas: Load<Persona[]>, products: Load<Product[]>, product: string, active: string | null): string | null {
  if (personas.state !== "ready") return null;
  const ready = (id: string | null | undefined) => id != null && personas.value.some((p) => p.ready && p.id === id);
  const fallback = products.state === "ready" ? products.value.find((p) => p.id === product)?.default_persona : null;
  if (ready(fallback)) return fallback!;
  if (ready(active)) return active;
  return personas.value.find((p) => p.ready)?.id ?? null;
}

/** Each repo by name, started from its URL's `origin/HEAD`. */
export function spawnRequest(product: string, persona: string, repos: string[], prompt: string): SpawnRequest {
  return { actor: "local", product, persona, repos: repos.map((repo) => ({ repo, local: null })), prompt, slug: null };
}
