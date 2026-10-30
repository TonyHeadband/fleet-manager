import { readFileSync } from "node:fs";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../api/generated/Agent";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DaemonSettings, Native } from "../native/native";
import { agent, DAEMON_SETTINGS, daemonMachine, fakeNative, RESTARTED } from "../test/fixtures";
import { click, openApp, pending, type AppOptions } from "../management/harness";
import { prLabels, sameFolder, typedLimits } from "./daemonSettings";

// Ports of the Settings → General daemon section tests in crates/fleet-gui/tests/settings.rs: the folders, limits,
// pull requests and agent defaults, each saved through the restart.

const css = readFileSync("src/settings/settings.css", "utf8");
const footer = () => screen.getByRole("contentinfo");
const field = (name: string) => screen.getByRole("textbox", { name });
const button = (name: string) => screen.getByRole("button", { name });
const dialog = () => screen.queryByRole("dialog", { name: "Restart the daemon?" });
const SAVE_PERSONAS = "Save personas folder";

/** The app on Settings → General, in the Tauri app. */
async function general(opts: AppOptions = {}) {
  const client = await openApp("Settings", null, { native: fakeNative(), ...opts });
  await waitFor(() => expect(field("Tasks folder")).toHaveValue("~/tasks"));
  return client;
}

async function retype(name: string, text: string) {
  await userEvent.clear(field(name));
  if (text) await userEvent.type(field(name), text);
}

beforeEach(() => localStorage.clear());

describe("The pure parts", () => {
  it("typed_limits_needs_whole_numbers_above_zero_and_lets_the_task_limits_be_empty", () => {
    expect(typedLimits(["8", "20", "3", "3", "", " "])).toEqual({ ...DAEMON_SETTINGS.limits });
    expect(typedLimits(["8", "20", "3", "3", "2", "1"])).toEqual({
      ...DAEMON_SETTINGS.limits,
      maxConcurrentTasks: 2,
      maxActiveProducts: 1,
    });
    for (const bad of ["0", "x", "", "1.5", "-1"]) expect(typedLimits(["8", bad, "3", "3", "", ""])).toBeNull();
    expect(typedLimits(["8", "20", "3", "3", "0", ""])).toBeNull();
    // egui parses a u32: past it, Save stays blocked instead of failing at the bridge.
    expect(typedLimits(["8", "4294967296", "3", "3", "", ""])).toBeNull();
    expect(typedLimits(["8", "4294967295", "3", "3", "", ""])?.maxSteps).toBe(4294967295);
  });

  it("pr_labels_split_on_commas_trimmed_without_empties", () => {
    expect(prLabels(" fleet, ,bot ")).toEqual(["fleet", "bot"]);
    expect(prLabels("")).toEqual([]);
  });

  it("same_folder_expands_the_tilde_and_ignores_a_trailing_slash", () => {
    expect(sameFolder("~/tasks/", "~/tasks", "/home/me")).toBe(true);
    expect(sameFolder("/home/me/tasks", "~/tasks", "/home/me")).toBe(true);
    expect(sameFolder(" ~/tasks ", "~/tasks", "/home/me")).toBe(true);
    expect(sameFolder("~/tasks2", "~/tasks", "/home/me")).toBe(false);
  });
});

describe("Tasks folder", () => {
  it("save_restarts_the_daemon", async () => {
    const daemonInfo = vi
      .fn()
      .mockResolvedValue({ address: "a", startedBy: "already running", stateDir: "/s", configFile: "/c", pid: null });
    const native = daemonMachine({ daemonInfo });
    const save = native.saveAndRestart;
    native.saveAndRestart = vi.fn(async (change) => {
      daemonInfo.mockResolvedValue({ address: "a", startedBy: "this window", stateDir: "/s", configFile: "/c", pid: "7" });
      return save(change);
    });
    await general({ native });
    await userEvent.type(field("Tasks folder"), "2");
    await userEvent.click(button("Save"));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "tasksRoot", text: "~/tasks2" });
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    await waitFor(() => expect(button("Save")).toBeDisabled());
    expect(button("Save")).toHaveAttribute("title", "This is the current tasks folder");
    await click("Daemon");
    expect(await screen.findByText("this window")).toBeInTheDocument();
  });

  it("save_is_disabled_for_the_current_root", async () => {
    await general();
    expect(button("Save")).toBeDisabled();
    expect(button("Save")).toHaveAttribute("title", "This is the current tasks folder");
    await userEvent.type(field("Tasks folder"), "/");
    expect(button("Save")).toBeDisabled();
    await userEvent.type(field("Tasks folder"), "x");
    expect(button("Save")).toBeEnabled();
    expect(button("Save")).not.toHaveAttribute("title");
  });

  it("save_is_disabled_while_an_agent_is_queued", async () => {
    const native = fakeNative();
    await general({ native, agents: [agent({ id: "q", status: "queued" }) as Partial<Agent>] });
    await userEvent.type(field("Tasks folder"), "2");
    expect(button("Save")).toBeDisabled();
    expect(button("Save")).toHaveAttribute("title", "An agent is still starting; wait until it runs");
  });

  it("an_edit_made_while_saving_survives_the_save", async () => {
    const held = pending<typeof RESTARTED>();
    const native = fakeNative({ saveAndRestart: vi.fn().mockReturnValue(held.promise) });
    vi.mocked(native.readDaemonSettings)
      .mockResolvedValueOnce(DAEMON_SETTINGS)
      .mockResolvedValue({ ...DAEMON_SETTINGS, tasksRoot: "~/tasks2" });
    await general({ native });
    await userEvent.type(field("Tasks folder"), "2");
    await userEvent.click(button("Save"));
    expect(screen.getAllByText("Restarting the daemon…").length).toBeGreaterThan(0);
    await userEvent.type(field("Tasks folder"), "3");
    await act(async () => held.resolve(RESTARTED));
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    expect(field("Tasks folder")).toHaveValue("~/tasks23");
  });

  it("an_unedited_folder_shows_the_saved_text_after_the_save", async () => {
    // egui's RootSaved: the field shows the folder as the config now has it, `~`-shortened.
    const native = daemonMachine();
    vi.mocked(native.readDaemonSettings)
      .mockResolvedValueOnce(DAEMON_SETTINGS)
      .mockResolvedValue({ ...DAEMON_SETTINGS, tasksRoot: "~/t2" });
    await general({ native });
    await retype("Tasks folder", "/home/me/t2");
    await userEvent.click(button("Save"));
    await waitFor(() => expect(field("Tasks folder")).toHaveValue("~/t2"));
  });

  it("save_with_a_running_agent_asks_first", async () => {
    const native = fakeNative();
    await general({ native, agents: [agent({ id: "w", status: "working" }) as Partial<Agent>] });
    await userEvent.type(field("Tasks folder"), "2");
    await userEvent.click(button("Save"));
    expect(within(dialog()!).getByText(/1 agent is running/)).toBeInTheDocument();
    expect(native.saveAndRestart).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Restart anyway" }));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "tasksRoot", text: "~/tasks2" });
  });

  it("a_daemon_without_a_pidfile_keeps_running_and_save_says_so", async () => {
    await general({
      native: fakeNative({ saveAndRestart: vi.fn().mockResolvedValue({ ...RESTARTED, outcome: "alreadyRunning", weStarted: false }) }),
    });
    await userEvent.type(field("Tasks folder"), "2");
    await userEvent.click(button("Save"));
    expect(await within(footer()).findByText("Saved. Restart the daemon to use the new folder.")).toBeInTheDocument();
  });

  it("browse_puts_the_picked_folder_in_the_field", async () => {
    const native = fakeNative({ pickFolder: vi.fn().mockResolvedValue("/home/me/elsewhere") });
    await general({ native });
    await userEvent.click(screen.getAllByRole("button", { name: "Browse…" })[0]!);
    expect(native.pickFolder).toHaveBeenCalledWith("~/tasks");
    await waitFor(() => expect(field("Tasks folder")).toHaveValue("/home/me/elsewhere"));
  });
});

describe("Personas folder", () => {
  it("personas_save_restarts_the_daemon", async () => {
    const native = daemonMachine();
    await general({ native });
    await userEvent.type(field("Personas folder"), "2");
    await userEvent.click(button(SAVE_PERSONAS));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "personasDir", text: "~/personas2" });
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    await waitFor(() => expect(button(SAVE_PERSONAS)).toHaveAttribute("title", "This is the current personas folder"));
  });

  it("personas_save_is_disabled_for_the_current_folder", async () => {
    await general();
    expect(button(SAVE_PERSONAS)).toBeDisabled();
    expect(button(SAVE_PERSONAS)).toHaveAttribute("title", "This is the current personas folder");
  });

  it("personas_save_is_disabled_while_an_agent_is_queued", async () => {
    await general({ agents: [agent({ id: "q", status: "queued" }) as Partial<Agent>] });
    await userEvent.type(field("Personas folder"), "2");
    expect(button(SAVE_PERSONAS)).toHaveAttribute("title", "An agent is still starting; wait until it runs");
  });

  it("personas_save_with_a_pending_request_asks_first", async () => {
    await general({ agents: [agent({ id: "r", status: "needs_you", repo_request: { repo: "portal", why: "login" } }) as Partial<Agent>] });
    await userEvent.type(field("Personas folder"), "2");
    await userEvent.click(button(SAVE_PERSONAS));
    expect(within(dialog()!).getByText(/Saving restarts the daemon, which ends that request/)).toBeInTheDocument();
  });

  it("personas_note_says_existing_folders_are_not_moved", async () => {
    await general();
    expect(
      screen.getByText(
        "Saving restarts the daemon. Existing personas' folders, with their Claude and GitHub logins, are not moved: move them by hand or log the personas in again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Applies to new tasks only. Saving restarts the daemon; running agents keep going.")).toBeInTheDocument();
  });

  it("an_empty_personas_folder_is_refused", async () => {
    const native = fakeNative({ saveAndRestart: vi.fn().mockRejectedValue(new Error("Enter a folder for the personas")) });
    await general({ native });
    await retype("Personas folder", "");
    await userEvent.click(button(SAVE_PERSONAS));
    expect(await within(footer()).findByText("Enter a folder for the personas")).toBeInTheDocument();
    await waitFor(() => expect(button(SAVE_PERSONAS)).toBeEnabled());
    expect(field("Personas folder")).toHaveValue("");
  });

  it("an_edit_made_while_saving_personas_survives_the_save", async () => {
    const held = pending<typeof RESTARTED>();
    const native = fakeNative({ saveAndRestart: vi.fn().mockReturnValue(held.promise) });
    await general({ native });
    await userEvent.type(field("Personas folder"), "2");
    await userEvent.click(button(SAVE_PERSONAS));
    await userEvent.type(field("Personas folder"), "3");
    await act(async () => held.resolve(RESTARTED));
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    expect(field("Personas folder")).toHaveValue("~/personas23");
  });
});

describe("Limits", () => {
  it("settings_saves_the_limits", async () => {
    const native = fakeNative();
    await general({ native });
    expect(button("Save limits")).toHaveAttribute("title", "These are the current limits");
    expect(field("Max steps")).toHaveAttribute("title", "Most steps one breakdown may declare.");
    await retype("Max steps", "30");
    await userEvent.type(field("Max concurrent tasks"), "2");
    await userEvent.click(button("Save limits"));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "limits", ...DAEMON_SETTINGS.limits, maxSteps: 30, maxConcurrentTasks: 2 });
    expect(
      screen.getByText(
        "Saving restarts the daemon; running agents keep going. A product can override each limit except Max active products on Management, Products.",
      ),
    ).toBeInTheDocument();
  });

  it("a_limit_that_is_not_a_positive_number_cannot_be_saved", async () => {
    const native = fakeNative();
    await general({ native });
    for (const bad of ["0", "x", ""]) {
      await retype("Max steps", bad);
      expect(button("Save limits")).toBeDisabled();
      expect(button("Save limits")).toHaveAttribute(
        "title",
        "Each limit is a whole number above zero; the task limits may be empty for no cap",
      );
    }
    expect(native.saveAndRestart).not.toHaveBeenCalled();
  });

  it("task_limits_empty_means_no_cap", async () => {
    await general();
    for (const name of ["Max concurrent tasks", "Max active products"]) {
      expect(field(name)).toHaveValue("");
      await retype(name, "0");
      expect(button("Save limits")).toBeDisabled();
      await retype(name, "2");
      expect(button("Save limits")).toBeEnabled();
      await retype(name, "");
      expect(button("Save limits")).toHaveAttribute("title", "These are the current limits");
    }
  });
});

describe("Pull requests and Agents", () => {
  const style = (name: string, error: string | null = null) =>
    ({ kind: "pr_style", name, summary: "", builtin: true, overridden: false, error }) as DefInfo;

  it("settings_saves_the_pr_keys", async () => {
    const native = fakeNative();
    await general({
      native,
      over: { definitions: vi.fn().mockResolvedValue([style("plain"), style("conventional"), style("broken", "broken.md: no title")]) },
    });
    const styles = screen.getByRole("combobox", { name: "Pull request style" });
    await waitFor(() =>
      expect(
        within(styles)
          .getAllByRole("option")
          .map((o) => o.textContent),
      ).toEqual(["plain", "conventional"]),
    );
    expect(button("Save pull requests")).toHaveAttribute("title", "These are the current pull request settings");
    await userEvent.selectOptions(styles, "conventional");
    await userEvent.type(field("Attribution"), "  Made by Fleet  ");
    await userEvent.click(screen.getByRole("checkbox", { name: "Draft pull requests" }));
    await userEvent.type(field("PR labels"), " fleet, ,bot ");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Gate build" }), "clean");
    expect(screen.getByRole("combobox", { name: "Gate build" })).toHaveAttribute(
      "title",
      "shared: the gate builds in the task's folder, reusing what the agents built. clean: it builds in a fresh folder that is removed after.",
    );
    await userEvent.click(button("Save pull requests"));
    expect(native.saveAndRestart).toHaveBeenCalledWith({
      kind: "pr",
      style: "conventional",
      attribution: "Made by Fleet",
      draft: true,
      labels: ["fleet", "bot"],
      gateBuild: "clean",
    });
  });

  it("an_unlisted_style_is_still_shown", async () => {
    await general({ over: { definitions: vi.fn().mockResolvedValue([style("conventional")]) } });
    expect(screen.getByRole("combobox", { name: "Pull request style" })).toHaveValue("plain");
  });

  it("settings_saves_the_default_effort", async () => {
    const native = fakeNative();
    await general({ native });
    expect(button("Save agent defaults")).toHaveAttribute("title", "These are the current defaults");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Default effort" }), "high");
    await userEvent.click(button("Save agent defaults"));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "agentDefaults", mode: "auto", effort: "high" });
    expect(screen.getByRole("combobox", { name: "Default mode" })).toHaveValue("auto");
  });
});

describe("In a browser", () => {
  it("in_a_browser_the_daemon_settings_say_only_in_the_app", async () => {
    await openApp("Settings", null, {});
    for (const [heading, save] of [
      ["Tasks folder", "Save"],
      ["Personas folder", SAVE_PERSONAS],
      ["Limits", "Save limits"],
      ["Pull requests", "Save pull requests"],
      ["Agents", "Save agent defaults"],
    ]) {
      const section = screen.getByRole("heading", { name: heading }).closest("section")!;
      expect(within(section).getByText("Only in the app")).toBeInTheDocument();
      expect(within(section).getByRole("button", { name: save })).toBeDisabled();
      expect(within(section).getByRole("button", { name: save })).toHaveAttribute("title", "Only in the app");
    }
  });
});

describe("The daemon sections' CSS", () => {
  it("the_daemon_sections_css_uses_tokens_only", () => {
    const rules = css.slice(css.indexOf("/* egui's general.rs daemon sections"));
    expect(rules.length).toBeLessThan(css.length);
    for (const literal of rules.match(/\b\d+(\.\d+)?(px|rem|em)\b/g) ?? []) expect(literal).toBe("1px");
    for (const token of ["--settings-folder-row", "--settings-limit-field", "--settings-pr-field"])
      expect(rules).toContain(`var(${token})`);
  });
});
