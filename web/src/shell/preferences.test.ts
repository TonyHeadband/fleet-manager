import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_CHOICES } from "../native/native";
import { fakeNative } from "../test/fixtures";
import { setChoiceStore } from "./choices";
import { applyPreferences, DEFAULT_PREFERENCES, MONO_FONTS, readPreferences, savePreference, UI_FONTS } from "./preferences";

// The preferences egui keeps in gui.toml (crates/fleet-client/src/config.rs's GuiSettings), and how the window takes them.

const root = document.documentElement;

beforeEach(() => {
  localStorage.clear();
  root.removeAttribute("data-theme");
  root.removeAttribute("data-animations");
  root.removeAttribute("style");
});
afterEach(() => {
  setChoiceStore(null);
  vi.restoreAllMocks();
});

describe("preferences", () => {
  it("saved_fonts_apply_at_startup", () => {
    setChoiceStore({ native: fakeNative(), saved: { ...DEFAULT_CHOICES, uiFont: "atkinson", monoFont: "atkinson" } });
    applyPreferences(readPreferences());
    expect(root.style.getPropertyValue("--font-sans")).toContain('"Atkinson Hyperlegible Next"');
    expect(root.style.getPropertyValue("--font-mono")).toContain('"Atkinson Hyperlegible Mono"');
  });

  it("system_theme_sets_no_data_theme", () => {
    applyPreferences({ ...DEFAULT_PREFERENCES, theme: "dark" });
    expect(root.dataset.theme).toBe("dark");
    applyPreferences({ ...DEFAULT_PREFERENCES, theme: "system" });
    expect(root.hasAttribute("data-theme")).toBe(false);
  });

  it("animations_off_marks_the_root", () => {
    applyPreferences({ ...DEFAULT_PREFERENCES, animations: false });
    expect(root.dataset.animations).toBe("off");
    applyPreferences(DEFAULT_PREFERENCES);
    expect(root.hasAttribute("data-animations")).toBe(false);
  });

  it("unknown_saved_values_read_as_the_defaults", () => {
    setChoiceStore({
      native: fakeNative(),
      saved: { ...DEFAULT_CHOICES, theme: "sepia", uiFont: "comic", monoFont: "courier", closeChoice: "explode" },
    });
    expect(readPreferences()).toEqual(DEFAULT_PREFERENCES);
    setChoiceStore(null);
    localStorage.setItem("fleet.theme", "sepia");
    localStorage.setItem("fleet.animations", "maybe");
    expect(readPreferences()).toEqual(DEFAULT_PREFERENCES);
  });

  it("a_browser_keeps_them_in_local_storage", async () => {
    await savePreference("theme", "light");
    await savePreference("animations", false);
    await savePreference("closeChoice", "stop_daemon");
    expect(localStorage.getItem("fleet.theme")).toBe("light");
    expect(readPreferences()).toEqual({ ...DEFAULT_PREFERENCES, theme: "light", animations: false, closeChoice: "stop_daemon" });
    await savePreference("closeChoice", null);
    expect(localStorage.getItem("fleet.closeChoice")).toBeNull();
  });

  it("the_app_saves_them_to_gui_toml_and_says_why_not", async () => {
    const native = fakeNative({ writeChoice: vi.fn().mockRejectedValue(new Error("read-only file system")) });
    setChoiceStore({ native, saved: DEFAULT_CHOICES });
    await expect(savePreference("uiFont", "space_grotesk")).rejects.toThrow("read-only file system");
    expect(native.writeChoice).toHaveBeenCalledWith("fleet.uiFont", "space_grotesk");
    // Not saved, but it holds for this window.
    expect(readPreferences().uiFont).toBe("space_grotesk");
    await savePreference("stickyAlerts", false).catch(() => {});
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.stickyAlerts", "false");
  });

  it("blocked_storage_keeps_the_choice_for_the_page", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    await expect(savePreference("theme", "dark")).resolves.toBeUndefined();
    expect(readPreferences().theme).toBe("dark");
  });

  it("a_chosen_theme_draws_native_controls_in_it_too", () => {
    // Checkboxes, selects and scrollbars follow color-scheme, not the page's colours.
    const css = readFileSync("src/generated/theme.css", "utf8");
    expect(css).toMatch(/:root\[data-theme="light"\] \{[^}]*color-scheme: light;/);
    expect(css).toMatch(/:root\[data-theme="dark"\] \{[^}]*color-scheme: dark;/);
    expect(css).toMatch(/:root:not\(\[data-theme="light"\]\) \{[^}]*color-scheme: dark;/);
  });

  it("every_font_has_its_face_in_the_theme_css", () => {
    const css = readFileSync("src/generated/theme.css", "utf8");
    for (const f of [...UI_FONTS, ...MONO_FONTS]) expect(css).toContain(`font-family: "${f.family}"`);
    expect(UI_FONTS.map((f) => f.label)).toEqual(["IBM Plex Sans", "Space Grotesk", "Atkinson Hyperlegible Next"]);
    expect(MONO_FONTS.map((f) => f.label)).toEqual(["IBM Plex Mono", "JetBrains Mono", "Atkinson Hyperlegible Mono"]);
  });
});
