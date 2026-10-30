import { afterEach, describe, expect, it, vi } from "vitest";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { fakeNative } from "../test/fixtures";
import { DEFAULT_CHOICES } from "../native/native";
import { activeProduct, newTaskBlocker, readChoice, readFleetDisplay, spawnPersona, spawnRequest, setChoiceStore, writeChoice, type Load } from "./choices";

const persona = (id: string, ready = true) => ({ id, ready }) as Persona;
const product = (id: string, default_persona: string | null = null) => ({ id, default_persona }) as Product;
const ready = <T>(value: T): Load<T> => ({ state: "ready", value });
const loading = { state: "loading" } as const;
const failed = { state: "failed" } as const;

afterEach(() => vi.restoreAllMocks());

describe("activeProduct", () => {
  it("keeps a saved product the list has", () => {
    expect(activeProduct(ready([product("p1")]), "p1")).toBe("p1");
  });
  it("drops a saved product the list no longer has", () => {
    expect(activeProduct(ready([product("p1")]), "gone")).toBeNull();
  });
  it("trusts a saved product until the list arrives", () => {
    expect(activeProduct(loading, "p1")).toBe("p1");
    expect(activeProduct(failed, "p1")).toBe("p1");
  });
});

describe("newTaskBlocker", () => {
  const products = ready([product("p1")]);
  it("waits for the personas", () => {
    expect(newTaskBlocker(loading, products, "p1")).toBe("Loading personas…");
    expect(newTaskBlocker(failed, products, "p1")).toBe("Could not load the personas; retrying");
  });
  it("needs a ready persona", () => {
    expect(newTaskBlocker(ready([persona("me", false)]), products, "p1")).toBe("Set up a persona first");
  });
  it("needs a product", () => {
    const personas = ready([persona("me")]);
    expect(newTaskBlocker(personas, loading, "p1")).toBe("Loading products…");
    expect(newTaskBlocker(personas, ready([]), null)).toBe("Create a product first");
    expect(newTaskBlocker(personas, products, null)).toBe("Pick a product");
    expect(newTaskBlocker(personas, products, "gone")).toBe("Pick a product");
  });
  it("lets a saved product through while the products cannot load", () => {
    const personas = ready([persona("me")]);
    expect(newTaskBlocker(personas, failed, "p1")).toBeNull();
    expect(newTaskBlocker(personas, failed, null)).toBe("Could not load the products; retrying");
  });
  it("is clear with a ready persona and a listed product", () => {
    expect(newTaskBlocker(ready([persona("me")]), products, "p1")).toBeNull();
  });
});

describe("spawnPersona", () => {
  const personas = ready([persona("off", false), persona("me"), persona("bot")]);
  it("prefers the product's default persona", () => {
    expect(spawnPersona(personas, ready([product("p1", "bot")]), "p1", "me")).toBe("bot");
  });
  it("then the active persona", () => {
    expect(spawnPersona(personas, ready([product("p1", "off")]), "p1", "bot")).toBe("bot");
  });
  it("then the first ready one", () => {
    expect(spawnPersona(personas, ready([product("p1")]), "p1", "off")).toBe("me");
  });
});

describe("saved choices", () => {
  it("read nothing when storage throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(readChoice("fleet.product")).toBeNull();
  });
  it("write nothing, quietly, when storage throws", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(() => writeChoice("fleet.product", "p1")).not.toThrow();
  });
  it("round-trip and clear", () => {
    writeChoice("fleet.persona", "me");
    expect(readChoice("fleet.persona")).toBe("me");
    writeChoice("fleet.persona", null);
    expect(readChoice("fleet.persona")).toBeNull();
  });
});

describe("spawnRequest", () => {
  it("names the product, the persona and each repo by name from its URL", () => {
    expect(spawnRequest("p1", "me", ["core"], "go")).toEqual({
      actor: "local",
      product: "p1",
      persona: "me",
      repos: [{ repo: "core", local: null }],
      prompt: "go",
      slug: null,
    });
  });
});

describe("readFleetDisplay", () => {
  it("an_unknown_display_reads_as_compact", () => {
    localStorage.removeItem("fleet.display");
    expect(readFleetDisplay()).toBe("compact");
    localStorage.setItem("fleet.display", "sideways");
    expect(readFleetDisplay()).toBe("compact");
    localStorage.setItem("fleet.display", "show");
    expect(readFleetDisplay()).toBe("show");
    localStorage.removeItem("fleet.display");
  });
});

describe("choices in the app", () => {
  afterEach(() => setChoiceStore(null));

  it("the_app_keeps_the_choices_in_gui_toml", () => {
    const native = fakeNative();
    setChoiceStore({ native, saved: { ...DEFAULT_CHOICES, persona: "work", display: "hide" } });
    expect(readChoice("fleet.persona")).toBe("work");
    expect(readFleetDisplay()).toBe("hide");
    writeChoice("fleet.product", "pie");
    expect(native.writeChoice).toHaveBeenCalledWith("fleet.product", "pie");
    expect(readChoice("fleet.product")).toBe("pie");
    expect(localStorage.getItem("fleet.product")).toBeNull();
  });
});
