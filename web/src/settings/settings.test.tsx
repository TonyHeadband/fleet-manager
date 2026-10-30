import { readFileSync } from "node:fs";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_CHOICES, type Native, type SavedChoices } from "../native/native";
import { agent, fakeNative } from "../test/fixtures";
import { click, openApp, type AppOptions } from "../management/harness";

// Ports of the Settings → General tests in crates/fleet-gui/tests/{shell_layout,shell,app,update,shortcuts}.rs.

const root = document.documentElement;
const css = readFileSync("src/settings/settings.css", "utf8");

const selectedTab = () => screen.getByRole("tab", { selected: true }).textContent;
const chip = (name: string) => screen.getByRole("button", { name });
const pressed = (name: string) => chip(name).getAttribute("aria-pressed") === "true";
const footer = () => screen.getByRole("contentinfo");

/** The app on Settings → General, in the Tauri app when `native` is given. */
function settings(opts: AppOptions = {}) {
  return openApp("Settings", null, opts);
}

const inApp = (over: Partial<Native> = {}, saved: Partial<SavedChoices> = {}) =>
  fakeNative({ readChoices: vi.fn().mockResolvedValue({ ...DEFAULT_CHOICES, ...saved }), ...over });

beforeEach(() => {
  localStorage.clear();
  root.removeAttribute("data-theme");
  root.removeAttribute("data-animations");
  root.removeAttribute("style");
});

describe("Settings: the view", () => {
  it("settings_remembers_its_tab_while_the_app_runs", async () => {
    await settings();
    expect(within(screen.getByRole("banner")).getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["General", "Daemon", "Extras", "Log"]);
    expect(selectedTab()).toBe("General");
    await click("Log");
    await click("Kanban");
    await click("Settings");
    expect(selectedTab()).toBe("Log");
  });

  it("selection_survives_settings", async () => {
    await openApp("Kanban", null, { agents: [agent({ id: "a", status: "working" })] });
    await userEvent.click(screen.getByText("fix the tests"));
    expect(screen.getByRole("complementary", { name: "Detail" })).toBeInTheDocument();
    await click("Settings");
    expect(screen.queryByRole("complementary", { name: "Detail" })).toBeNull();
    await click("Kanban");
    expect(screen.getByRole("complementary", { name: "Detail" })).toBeInTheDocument();
  });

  it("new_task_is_there_on_settings_too", async () => {
    localStorage.setItem("fleet.product", "a");
    await settings();
    await waitFor(() => expect(screen.getByRole("button", { name: "New task" })).toBeEnabled());
    await click("New task");
    expect(await screen.findByRole("button", { name: "Start task" })).toBeInTheDocument();
  });

  it("the_tabs_not_ported_yet_say_so", async () => {
    await settings();
    for (const tab of ["Daemon", "Extras", "Log"]) {
      await click(tab);
      expect(screen.getByText("not on the web yet")).toBeInTheDocument();
    }
  });
});

describe("Settings: preferences", () => {
  it("theme_choice_saves_and_applies", async () => {
    const native = inApp();
    await settings({ native });
    expect(screen.getByText("Theme")).toBeInTheDocument();
    expect(pressed("System")).toBe(true);
    await userEvent.click(chip("Dark"));
    expect(root.dataset.theme).toBe("dark");
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.theme", "dark");
    expect(await within(footer()).findByText("Settings saved")).toBeInTheDocument();
    expect(pressed("Dark")).toBe(true);
    await userEvent.click(chip("System"));
    expect(root.hasAttribute("data-theme")).toBe(false);
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.theme", "system");
  });

  it("font_choices_save_and_apply_independently", async () => {
    const native = inApp();
    await settings({ native });
    expect(screen.getByText("Font")).toBeInTheDocument();
    expect(screen.getByText("Mono font")).toBeInTheDocument();
    // Each chip shows its name in its own face.
    expect(within(chip("Space Grotesk")).getByText("Space Grotesk")).toHaveStyle({ fontFamily: '"Space Grotesk"' });
    await userEvent.click(chip("Space Grotesk"));
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.uiFont", "space_grotesk");
    expect(root.style.getPropertyValue("--font-sans")).toContain("Space Grotesk");
    expect(root.style.getPropertyValue("--font-mono")).toContain("IBM Plex Mono");
    await userEvent.click(chip("JetBrains Mono"));
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.monoFont", "jetbrains");
    expect(root.style.getPropertyValue("--font-sans")).toContain("Space Grotesk");
    await userEvent.click(chip("IBM Plex Sans"));
    expect(root.style.getPropertyValue("--font-sans")).toContain("IBM Plex Sans");
    expect(root.style.getPropertyValue("--font-mono")).toContain("JetBrains Mono");
  });

  it("a_click_on_the_chosen_chip_saves_nothing", async () => {
    const native = inApp();
    await settings({ native });
    await userEvent.click(chip("System"));
    expect(native.writeChoice).not.toHaveBeenCalledWith("fleet.theme", expect.anything());
  });

  it("animations_switch_saves_and_applies", async () => {
    const native = inApp();
    await settings({ native });
    expect(screen.getByText("Motion")).toBeInTheDocument();
    const box = screen.getByRole("checkbox", { name: "Animations" });
    expect(box).toBeChecked();
    expect(box.closest("label")).toHaveAttribute("title", "Card edges, the radar and other motion. Off keeps the window idle.");
    await userEvent.click(box);
    expect(root.dataset.animations).toBe("off");
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.animations", "false");
    await userEvent.click(box);
    expect(root.hasAttribute("data-animations")).toBe(false);
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.animations", "true");
  });

  it("the_footer_checkbox_is_on_by_default_and_saved_at_once", async () => {
    const native = inApp();
    await settings({ native });
    expect(screen.getByText("Footer")).toBeInTheDocument();
    const box = screen.getByRole("checkbox", { name: "Keep errors and warnings in the footer until dismissed" });
    expect(box).toBeChecked();
    await userEvent.click(box);
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.stickyAlerts", "false");
  });

  it("a_failed_save_still_applies_and_says_so", async () => {
    const native = inApp({ writeChoice: vi.fn().mockRejectedValue(new Error("read-only file system")) });
    await settings({ native });
    await userEvent.click(chip("Dark"));
    expect(root.dataset.theme).toBe("dark");
    expect(await within(footer()).findByText("Could not save the theme: read-only file system")).toBeInTheDocument();
  });

  it("a_browser_keeps_the_preferences_in_local_storage", async () => {
    await settings();
    await userEvent.click(chip("Light"));
    expect(root.dataset.theme).toBe("light");
    expect(localStorage.getItem("fleet.theme")).toBe("light");
    expect(await within(footer()).findByText("Settings saved")).toBeInTheDocument();
  });
});

describe("Settings: closing the window", () => {
  it("general_lists_the_close_choices", async () => {
    await settings({ native: inApp() });
    expect(screen.getByText("Closing the window")).toBeInTheDocument();
    for (const name of ["Ask", "Keep daemon running", "Stop daemon"]) expect(chip(name)).toBeEnabled();
    expect(pressed("Ask")).toBe(true);
  });

  it("close_choice_ask_clears_the_saved_choice", async () => {
    const native = inApp({}, { closeChoice: "stop_daemon" });
    await settings({ native });
    expect(pressed("Stop daemon")).toBe(true);
    await userEvent.click(chip("Ask"));
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.closeChoice", null);
  });

  it("a_close_choice_made_in_settings_is_used_without_asking", async () => {
    // The window's close reads gui.toml as it closes (crates/fleet-tauri/src/close.rs): saving is all it takes.
    const native = inApp();
    await settings({ native });
    await userEvent.click(chip("Keep daemon running"));
    expect(native.writeChoice).toHaveBeenLastCalledWith("fleet.closeChoice", "keep_running");
  });

  it("a_choice_remembered_by_the_close_dialog_shows_in_settings_at_once", async () => {
    const readChoices = vi.fn().mockResolvedValue(DEFAULT_CHOICES);
    await settings({ native: inApp({ readChoices }) });
    expect(pressed("Ask")).toBe(true);
    await click("Kanban");
    // The close dialog's "Remember my choice" wrote it meanwhile.
    readChoices.mockResolvedValue({ ...DEFAULT_CHOICES, closeChoice: "stop_daemon" });
    await click("Settings");
    await waitFor(() => expect(pressed("Stop daemon")).toBe(true));
  });

  it("the_theme_button_is_gone", async () => {
    await settings();
    expect(screen.queryByText(/Theme:/)).toBeNull();
  });
});

describe("Settings: version", () => {
  const release = { version: "99.0.0", notes: "", page: "https://example.invalid/r/99" };

  it("a_newer_release_shows_the_modal", async () => {
    const native = inApp({ checkUpdate: vi.fn().mockResolvedValue({ kind: "available", ...release, skipped: false }) });
    await settings({ native });
    expect(await screen.findByText("v99.0.0 available")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /is available/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    await userEvent.click(screen.getByRole("link", { name: "Download" }));
    expect(native.openUrl).toHaveBeenCalledWith(release.page);
  });

  it("skip_this_version_hides_it_next_time", async () => {
    await settings({ native: inApp({ checkUpdate: vi.fn().mockResolvedValue({ kind: "available", ...release, skipped: true }) }) });
    expect(await screen.findByText("v99.0.0 available")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
  });

  it("a_failed_check_only_changes_the_settings_line", async () => {
    await settings({ native: inApp({ checkUpdate: vi.fn().mockResolvedValue({ kind: "failed" }) }) });
    expect(await screen.findByText("Could not check for updates")).toBeInTheDocument();
    expect(within(footer()).queryByText(/update/i)).toBeNull();
  });

  it("an_answered_check_says_up_to_date_in_settings", async () => {
    await settings({ native: inApp({ checkUpdate: vi.fn().mockResolvedValue({ kind: "current" }) }) });
    expect(await screen.findByText("Version v0.2.0 — up to date")).toBeInTheDocument();
  });

  it("before_a_check_answers_the_version_stands_alone", async () => {
    await settings({ native: inApp() });
    expect(await screen.findByText("Version v0.2.0")).toBeInTheDocument();
  });

  it("a_browser_shows_the_daemon_s_version", async () => {
    await settings({ over: { version: vi.fn().mockResolvedValue({ version: "0.4.1" }) } });
    expect(await screen.findByText("Version v0.4.1")).toBeInTheDocument();
  });
});

describe("Settings: shortcuts", () => {
  const reply = (state: "none" | "current" | "stale", menu = false, desktop = false) => ({ state, menu, desktop });

  it("settings_adds_and_removes_the_ticked_places", async () => {
    const changeShortcuts = vi
      .fn()
      .mockResolvedValueOnce(reply("current", true))
      .mockResolvedValueOnce(reply("current", true, true))
      .mockResolvedValueOnce(reply("current", true));
    const native = inApp({ changeShortcuts });
    await settings({ native });
    expect(await screen.findByText("No shortcuts yet")).toBeInTheDocument();
    const menu = screen.getByRole("checkbox", { name: "Start menu" });
    const desktop = screen.getByRole("checkbox", { name: "Desktop" });
    expect(menu).toBeChecked();
    expect(desktop).not.toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "Add shortcuts" }));
    expect(changeShortcuts).toHaveBeenLastCalledWith("add", true, false);
    expect(await screen.findByText("Shortcuts in: Start menu")).toBeInTheDocument();
    await userEvent.click(desktop);
    await userEvent.click(screen.getByRole("button", { name: "Add shortcuts" }));
    expect(await screen.findByText("Shortcuts in: Start menu, Desktop")).toBeInTheDocument();
    await userEvent.click(menu);
    await userEvent.click(screen.getByRole("button", { name: "Remove shortcuts" }));
    expect(changeShortcuts).toHaveBeenLastCalledWith("remove", false, true);
    expect(await screen.findByText("Shortcuts in: Start menu")).toBeInTheDocument();
    await userEvent.click(desktop);
    expect(screen.getByRole("button", { name: "Add shortcuts" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove shortcuts" })).toBeDisabled();
  });

  it("an_old_location_offers_update", async () => {
    const changeShortcuts = vi.fn().mockResolvedValue(reply("current", true));
    await settings({ native: inApp({ shortcuts: vi.fn().mockResolvedValue(reply("stale", true)), changeShortcuts }) });
    expect(await screen.findByText("Shortcuts point to an old location —")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Update" }));
    expect(changeShortcuts).toHaveBeenCalledWith("update", true, false);
    await waitFor(() => expect(screen.queryByText("Shortcuts point to an old location —")).toBeNull());
    expect(screen.getByText("Shortcuts in: Start menu")).toBeInTheDocument();
  });

  it("a_shortcut_change_that_fails_says_why", async () => {
    await settings({ native: inApp({ changeShortcuts: vi.fn().mockRejectedValue(new Error("permission denied")) }) });
    await userEvent.click(await screen.findByRole("button", { name: "Add shortcuts" }));
    expect(await screen.findByText("Could not change the shortcuts: permission denied")).toBeInTheDocument();
  });
});

describe("Settings: in a browser", () => {
  it("in_a_browser_shortcuts_and_closing_say_only_in_the_app", async () => {
    await settings();
    for (const name of ["Ask", "Keep daemon running", "Stop daemon", "Add shortcuts", "Remove shortcuts"]) {
      expect(chip(name)).toBeDisabled();
      expect(chip(name)).toHaveAttribute("title", "Only in the app");
    }
    // The preferences that are the page's own still work.
    for (const name of ["Dark", "Space Grotesk", "JetBrains Mono"]) expect(chip(name)).toBeEnabled();
  });

  it("the_settings_css_uses_tokens_only_and_the_font_chips_wrap", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).toMatch(/\.settings-page \.chips \{[^}]*flex-wrap: wrap/);
    // egui starts the first heading at the page's own margin, with no section gap above it.
    expect(css).toMatch(/\.settings-page > \.section:first-child \{[^}]*margin-top: 0/);
  });
});
