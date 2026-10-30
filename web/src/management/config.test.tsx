import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { AgentConfig } from "../api/generated/AgentConfig";
import { click, management } from "./harness";

// Ports of the Instructions and Agent settings tests in crates/fleet-gui/tests/management.rs.

const INSTRUCTIONS = "Instructions for every agent";

const config = (instructions: string, over: Partial<AgentConfig> = {}): AgentConfig => ({
  instructions,
  settings: { allow: [], deny: [], env: {} },
  settings_error: null,
  ...over,
});

/** A promise answered by hand, as egui's tests hand results to the app. */
function pending<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** egui's `config_tab`: Management on `tab` with the daemon's config; saves wait to be answered by hand. */
async function configTab(tab: string, cfg: AgentConfig | Error) {
  const saves: ReturnType<typeof pending<AgentConfig>>[] = [];
  // As egui's harness: a failed load is the one injected; any asked after it is still out.
  const agentConfig =
    cfg instanceof Error
      ? vi
          .fn()
          .mockRejectedValueOnce(cfg)
          .mockReturnValue(new Promise(() => {}))
      : vi.fn().mockResolvedValue(cfg);
  const saveAgentConfig = vi.fn(() => {
    const p = pending<AgentConfig>();
    saves.push(p);
    return p.promise;
  });
  const client = await management(tab, { over: { agentConfig, saveAgentConfig } });
  return { client, saves, agentConfig, saveAgentConfig };
}

const field = (name: string) => screen.getByRole("textbox", { name });
const button = (name: string) => screen.getByRole("button", { name });
const enabled = (name: string) => !(button(name) as HTMLButtonElement).disabled;
const land = async <T,>(p: ReturnType<typeof pending<T>>, v: T) => act(async () => p.resolve(v));

beforeEach(() => localStorage.clear());

describe("Instructions and Agent settings", () => {
  it("opening_management_loads_the_config_once", async () => {
    const { agentConfig } = await configTab("Overview", config(""));
    await click("Instructions");
    await click("Agent settings");
    expect(agentConfig).toHaveBeenCalledTimes(1);
  });

  it("instructions_save_sends_text", async () => {
    const { saves, saveAgentConfig } = await configTab("Instructions", config("Be brief."));
    await screen.findByRole("textbox", { name: INSTRUCTIONS });
    expect(enabled("Save")).toBe(false);
    expect(enabled("Revert")).toBe(false);
    expect(screen.getByText("Applies to agents started after saving.")).toBeInTheDocument();
    await userEvent.type(field(INSTRUCTIONS), " Be kind.");
    expect(enabled("Save")).toBe(true);
    await userEvent.click(button("Save"));
    // Only the text goes: the settings file is not this tab's to rewrite.
    expect(saveAgentConfig).toHaveBeenLastCalledWith({ instructions: "Be brief. Be kind." });
    await land(saves[0]!, config("Be brief. Be kind."));
    expect(enabled("Save")).toBe(false);
  });

  it("revert_restores_saved", async () => {
    await configTab("Instructions", config("Be brief."));
    await userEvent.type(await screen.findByRole("textbox", { name: INSTRUCTIONS }), " Extra");
    await userEvent.click(button("Revert"));
    expect(field(INSTRUCTIONS)).toHaveValue("Be brief.");
    expect(enabled("Save")).toBe(false);
    await click("Agent settings");
    await userEvent.click(button("Add allow rule"));
    await userEvent.type(field("allow rule 1"), "Bash(ls)");
    await userEvent.click(button("Revert"));
    expect(screen.queryByRole("textbox", { name: "allow rule 1" })).toBeNull();
  });

  it("settings_rows_add_and_remove", async () => {
    const { saves, saveAgentConfig } = await configTab(
      "Agent settings",
      config("", { settings: { allow: [], deny: ["Read(./.env)"], env: {} } }),
    );
    expect(await screen.findByRole("textbox", { name: "deny rule 1" })).toBeInTheDocument();
    await userEvent.click(button("Add allow rule"));
    await userEvent.click(button("Add variable"));
    await userEvent.type(field("allow rule 1"), "Bash(ls)");
    await userEvent.type(field("Variable name 1"), "EDITOR");
    await userEvent.type(field("Variable value 1"), "vim");
    await userEvent.click(button("Save"));
    expect(saveAgentConfig).toHaveBeenLastCalledWith({ settings: { allow: ["Bash(ls)"], deny: ["Read(./.env)"], env: { EDITOR: "vim" } } });
    await land(saves[0]!, config("", { settings: { allow: ["Bash(ls)"], deny: ["Read(./.env)"], env: { EDITOR: "vim" } } }));
    await userEvent.click(button("Remove deny rule 1"));
    expect(screen.queryByRole("textbox", { name: "deny rule 1" })).toBeNull();
    await userEvent.click(button("Remove variable 1"));
    expect(screen.queryByRole("textbox", { name: "Variable name 1" })).toBeNull();
    expect(enabled("Save")).toBe(true);
  });

  it("duplicate_variable_is_a_local_error", async () => {
    const { saveAgentConfig } = await configTab("Agent settings", config(""));
    await screen.findByRole("button", { name: "Add variable" });
    await userEvent.click(button("Add variable"));
    await userEvent.click(button("Add variable"));
    await userEvent.type(field("Variable name 1"), "A");
    await userEvent.type(field("Variable name 2"), "A");
    await userEvent.click(button("Save"));
    expect(saveAgentConfig).not.toHaveBeenCalled();
    expect(screen.getByText(/A is set twice/)).toBeInTheDocument();
  });

  it("a_nameless_variable_with_a_value_is_a_local_error", async () => {
    const { saveAgentConfig } = await configTab("Agent settings", config(""));
    await userEvent.click(await screen.findByRole("button", { name: "Add variable" }));
    await userEvent.type(field("Variable value 1"), "x");
    await userEvent.click(button("Save"));
    expect(saveAgentConfig).not.toHaveBeenCalled();
    expect(screen.getByText("An environment variable needs a name.")).toBeInTheDocument();
  });

  it("server_rejection_shows_message_and_keeps_draft", async () => {
    const { saves } = await configTab("Agent settings", config(""));
    await userEvent.click(await screen.findByRole("button", { name: "Add variable" }));
    await userEvent.type(field("Variable name 1"), "GH_TOKEN");
    await userEvent.click(button("Save"));
    await act(async () => saves[0]!.reject(new FleetError("GH_TOKEN is reserved")));
    expect(screen.getByText(/GH_TOKEN is reserved/)).toBeInTheDocument();
    expect(field("Variable name 1")).toHaveValue("GH_TOKEN");
    expect(enabled("Save")).toBe(true);
  });

  it("leaving_the_tab_keeps_the_draft", async () => {
    await configTab("Instructions", config("a"));
    await userEvent.type(await screen.findByRole("textbox", { name: INSTRUCTIONS }), "b");
    await click("Personas");
    await click("Instructions");
    expect(field(INSTRUCTIONS)).toHaveValue("ab");
    // And leaving Management itself.
    await click("Kanban");
    await click("Management");
    expect(field(INSTRUCTIONS)).toHaveValue("ab");
  });

  it("a_broken_file_shows_its_message_and_stays_editable", async () => {
    const { agentConfig } = await configTab("Agent settings", new FleetError("settings.json is not valid JSON"));
    expect(await screen.findByText(/settings\.json is not valid JSON/)).toBeInTheDocument();
    await userEvent.click(button("Add allow rule"));
    await userEvent.type(field("allow rule 1"), "Bash(ls)");
    expect(enabled("Save")).toBe(true);
    // Re-entering the tab asks again.
    await click("Personas");
    await click("Agent settings");
    expect(agentConfig).toHaveBeenCalledTimes(2);
  });

  it("a_settings_save_after_a_settings_error_never_sends_instructions", async () => {
    const { saves, saveAgentConfig } = await configTab(
      "Agent settings",
      config("keep me", { settings_error: "cannot read settings.json: bad" }),
    );
    expect(await screen.findByText(/cannot read settings\.json: bad\. Saving valid settings replaces the file\./)).toBeInTheDocument();
    // Saving valid settings is how the file is repaired, so Save is on while Revert (nothing to undo) is not.
    expect(enabled("Save")).toBe(true);
    expect(enabled("Revert")).toBe(false);
    await userEvent.click(button("Add allow rule"));
    await userEvent.type(field("allow rule 1"), "Read");
    await userEvent.click(button("Save"));
    expect(saveAgentConfig).toHaveBeenLastCalledWith({ settings: { allow: ["Read"], deny: [], env: {} } });
    await land(saves[0]!, config("keep me", { settings: { allow: ["Read"], deny: [], env: {} } }));
    expect(screen.queryByText(/cannot read settings\.json/)).toBeNull();
  });

  it("the_instructions_tab_works_while_settings_are_broken", async () => {
    const { saveAgentConfig } = await configTab("Instructions", config("keep me", { settings_error: "settings.json is broken" }));
    await userEvent.type(await screen.findByRole("textbox", { name: INSTRUCTIONS }), "!");
    expect(screen.queryByText(/settings\.json is broken/)).toBeNull();
    await userEvent.click(button("Save"));
    expect(saveAgentConfig).toHaveBeenLastCalledWith({ instructions: "keep me!" });
  });

  it("a_saved_instructions_answer_keeps_a_settings_draft", async () => {
    const { saves } = await configTab("Agent settings", config(""));
    await userEvent.click(await screen.findByRole("button", { name: "Add allow rule" }));
    await click("Instructions");
    await userEvent.type(field(INSTRUCTIONS), "x");
    await userEvent.click(button("Save"));
    await land(saves[0]!, config("x"));
    await click("Agent settings");
    expect(field("allow rule 1")).toBeInTheDocument();
  });

  it("a_save_landing_after_moving_on_keeps_the_other_draft", async () => {
    const { saves } = await configTab("Instructions", config("a"));
    await userEvent.type(await screen.findByRole("textbox", { name: INSTRUCTIONS }), "b");
    await userEvent.click(button("Save"));
    // Moved on to the settings and typing there when the instructions save lands.
    await click("Agent settings");
    await userEvent.click(button("Add allow rule"));
    await userEvent.type(field("allow rule 1"), "Read");
    await land(saves[0]!, config("ab"));
    expect(field("allow rule 1")).toHaveValue("Read");
    await click("Instructions");
    expect(field(INSTRUCTIONS)).toHaveValue("ab");
    expect(enabled("Save")).toBe(false);
  });

  it("a_failed_config_load_says_so_and_retries_on_reopen", async () => {
    // Down for the first open and for the tab picked in it; back by the next way in.
    const agentConfig = vi
      .fn()
      .mockRejectedValueOnce(new FleetError("daemon down"))
      .mockRejectedValueOnce(new FleetError("daemon down"))
      .mockResolvedValue(config("back"));
    await management("Instructions", { over: { agentConfig } });
    expect(await screen.findByText("daemon down")).toBeInTheDocument();
    // The other tabs are still there.
    expect(screen.getByRole("tab", { name: "Overview" })).toBeInTheDocument();
    await click("Kanban");
    await click("Management");
    expect(await screen.findByRole("textbox", { name: INSTRUCTIONS })).toHaveValue("back");
    expect(screen.queryByText("daemon down")).toBeNull();
  });
});
