import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../api/generated/Agent";
import type { ToolList } from "../api/generated/ToolList";
import type { DaemonSettings, Native } from "../native/native";
import { agent, DAEMON_SETTINGS, daemonMachine, RESTARTED, toolList } from "../test/fixtures";
import { click, openApp, pending, type AppOptions } from "../management/harness";

// Ports of the relaunched wizard and pending-restart tests in crates/fleet-gui/tests/settings.rs.

const footer = () => screen.getByRole("contentinfo");
const wizard = () => screen.queryByRole("dialog", { name: /^Step \d of \d · / });
const field = (name: string) => screen.getByRole("textbox", { name });
const button = (name: string) => screen.getByRole("button", { name });
const PENDING = "Saved: Tasks folder ~/tasks2 — takes effect when the background service restarts.";
const TAKES_EFFECT = "The new folders are saved. They take effect when the background service next starts.";

/** The app on Settings → `tab`, its tool checks answered by `tools`. */
async function settings(tab: string | null, opts: AppOptions & { tools?: ToolList | ReturnType<typeof vi.fn> } = {}) {
  const { tools = toolList({ claude: true }), ...rest } = opts;
  const recheckTools = typeof tools === "function" ? tools : vi.fn().mockResolvedValue(tools);
  const native = rest.native === undefined ? daemonMachine() : rest.native;
  await openApp("Settings", tab, { ...rest, native, over: { recheckTools, ...rest.over } });
  return { native, recheckTools };
}

async function relaunch() {
  await userEvent.click(await screen.findByRole("button", { name: "Relaunch setup wizard" }));
  expect(await screen.findByRole("heading", { name: "Step 1 of 3 · What Fleet Manager needs" })).toBeInTheDocument();
}

/** From Requirements: Continue, `suffix` typed after the tasks folder, Next. */
async function changeTasksFolder(suffix: string) {
  await userEvent.click(await within(wizard()!).findByRole("button", { name: /^Continue/ }));
  if (suffix) await userEvent.type(within(wizard()!).getByRole("textbox", { name: "Tasks folder" }), suffix);
  await userEvent.click(within(wizard()!).getByRole("button", { name: "Next →" }));
}

beforeEach(() => localStorage.clear());

describe("The relaunched wizard", () => {
  it("relaunch_opens_the_wizard_at_requirements", async () => {
    const { recheckTools } = await settings(null);
    expect(recheckTools).not.toHaveBeenCalled();
    await relaunch();
    expect(recheckTools).toHaveBeenCalledTimes(1);
    expect(await within(wizard()!).findByText("Claude Code is not installed")).toBeInTheDocument();
    expect(within(wizard()!).getByText("What the background service found on this machine.")).toBeInTheDocument();
  });

  it("a_folder_changed_in_the_relaunched_wizard_is_saved_and_offers_restart_now", async () => {
    const { native } = await settings("Extras");
    await relaunch();
    await userEvent.click(await within(wizard()!).findByRole("button", { name: "Continue without it" }));
    expect(screen.getByRole("heading", { name: "Step 2 of 3 · Where Fleet Manager lives" })).toBeInTheDocument();
    expect(screen.getByText("A changed folder is saved now and used when the background service next starts.")).toBeInTheDocument();
    const tasks = within(wizard()!).getByRole("textbox", { name: "Tasks folder" });
    expect(tasks).toHaveValue("~/tasks");
    await userEvent.type(tasks, "2");
    await userEvent.click(button("Next →"));
    expect(await screen.findByRole("heading", { name: "Step 3 of 3 · Done" })).toBeInTheDocument();
    expect(native!.saveSettings).toHaveBeenCalledTimes(1);
    expect(native!.saveSettings).toHaveBeenCalledWith({ kind: "tasksRoot", text: "~/tasks2" });
    expect(native!.saveAndRestart).not.toHaveBeenCalled();
    expect(
      screen.getByText("Still missing: Claude Code. Check again from Settings → Extras, or relaunch this wizard."),
    ).toBeInTheDocument();
    expect(await screen.findByText(TAKES_EFFECT)).toBeInTheDocument();
    await userEvent.click(within(wizard()!).getByRole("button", { name: "Restart now" }));
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    expect(native!.saveAndRestart).toHaveBeenCalledWith({ kind: "restart" });
    expect(wizard()).toBeNull();
    await click("General");
    await waitFor(() => expect(field("Tasks folder")).toHaveValue("~/tasks2"));
  });

  it("the_done_step_comes_with_its_restart_line", async () => {
    // Done waits for the config to be read again, so it never shows without the pending line (#420).
    const native = daemonMachine();
    const reread = pending<DaemonSettings>();
    await settings("Extras", { native });
    vi.mocked(native.readDaemonSettings).mockReturnValueOnce(reread.promise);
    await relaunch();
    await changeTasksFolder("2");
    expect(screen.queryByRole("heading", { name: "Step 3 of 3 · Done" })).toBeNull();
    await act(async () => reread.resolve({ ...DAEMON_SETTINGS, tasksRoot: "~/tasks2" }));
    expect(await screen.findByRole("heading", { name: "Step 3 of 3 · Done" })).toBeInTheDocument();
    expect(screen.getByText(TAKES_EFFECT)).toBeInTheDocument();
  });

  it("a_wizard_opened_before_the_settings_load_fills_its_folders", async () => {
    const first = pending<DaemonSettings>();
    const native = daemonMachine();
    vi.mocked(native.readDaemonSettings).mockReturnValueOnce(first.promise);
    await settings("Extras", { native });
    await userEvent.click(await screen.findByRole("button", { name: "Relaunch setup wizard" }));
    await act(async () => first.resolve(DAEMON_SETTINGS));
    await userEvent.click(await within(wizard()!).findByRole("button", { name: /^Continue/ }));
    expect(within(wizard()!).getByRole("textbox", { name: "Tasks folder" })).toHaveValue("~/tasks");
    expect(within(wizard()!).getByRole("textbox", { name: "Personas folder" })).toHaveValue("~/personas");
  });

  it("the_relaunched_wizard_changing_nothing_just_closes", async () => {
    const { native } = await settings(null);
    await relaunch();
    await changeTasksFolder("");
    expect(await screen.findByRole("heading", { name: "Step 3 of 3 · Done" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart now" })).toBeNull();
    await userEvent.click(within(wizard()!).getByRole("button", { name: "Done" }));
    expect(wizard()).toBeNull();
    expect(native!.saveSettings).not.toHaveBeenCalled();
    expect(native!.saveAndRestart).not.toHaveBeenCalled();
  });

  it("the_done_step_waits_for_a_check_still_in_flight", async () => {
    const held = pending<ToolList>();
    await settings(null, { tools: vi.fn().mockReturnValue(held.promise) });
    await relaunch();
    expect(within(wizard()!).getByText("Checking the tools…")).toBeInTheDocument();
    await changeTasksFolder("");
    expect(await screen.findByText("Still checking the tools…")).toBeInTheDocument();
    await act(async () => held.resolve(toolList()));
    expect(screen.getByText("Everything Fleet Manager needs is there.")).toBeInTheDocument();
  });

  it("the_done_step_says_when_the_tools_could_not_be_checked", async () => {
    await settings(null, { tools: vi.fn().mockRejectedValue(new Error("connection refused")) });
    await relaunch();
    expect(await within(wizard()!).findByText("connection refused")).toBeInTheDocument();
    await changeTasksFolder("");
    expect(await screen.findByText("Unable to check the tools.")).toBeInTheDocument();
  });

  it("a_failed_folder_write_stays_on_folders_with_its_error", async () => {
    await settings(null, {
      native: daemonMachine({ saveSettings: vi.fn().mockRejectedValue(new Error("Cannot write /c/config.toml: denied")) }),
    });
    await relaunch();
    await changeTasksFolder("2");
    expect(await within(wizard()!).findByText("Cannot write /c/config.toml: denied")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Step 2 of 3 · Where Fleet Manager lives" })).toBeInTheDocument();
  });

  it("the_wizard_folders_next_waits_while_the_daemon_restarts", async () => {
    const held = pending<typeof RESTARTED>();
    await settings("Daemon", { native: daemonMachine({ saveAndRestart: vi.fn().mockReturnValue(held.promise) }) });
    await userEvent.click(button("Restart daemon"));
    await click("Extras");
    await relaunch();
    await userEvent.click(await within(wizard()!).findByRole("button", { name: /^Continue/ }));
    expect(button("Next →")).toBeDisabled();
    expect(button("Next →")).toHaveAttribute("title", "The daemon is restarting");
    await act(async () => held.resolve(RESTARTED));
  });

  it("in_a_browser_the_wizard_has_no_folders_step", async () => {
    await settings(null, { native: null });
    await userEvent.click(await screen.findByRole("button", { name: "Relaunch setup wizard" }));
    expect(await screen.findByRole("heading", { name: "Step 1 of 2 · What Fleet Manager needs" })).toBeInTheDocument();
    await userEvent.click(await within(wizard()!).findByRole("button", { name: /^Continue/ }));
    expect(screen.getByRole("heading", { name: "Step 2 of 2 · Done" })).toBeInTheDocument();
  });

  it("escape_closes_the_wizard", async () => {
    await settings(null);
    await relaunch();
    await userEvent.keyboard("{Escape}");
    expect(wizard()).toBeNull();
  });
});

describe("The pending restart in General", () => {
  it("a_folder_the_wizard_saved_waits_for_the_restart_in_general", async () => {
    const { native } = await settings(null);
    await relaunch();
    await changeTasksFolder("2");
    await userEvent.click(await within(wizard()!).findByRole("button", { name: "Done" }));
    // The daemon still runs with the old folder: it stays the current one.
    expect(field("Tasks folder")).toHaveValue("~/tasks");
    expect(button("Save")).toBeDisabled();
    expect(await screen.findByText(PENDING)).toBeInTheDocument();
    expect(native!.saveAndRestart).not.toHaveBeenCalled();
    await userEvent.click(button("Restart now"));
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    await waitFor(() => expect(field("Tasks folder")).toHaveValue("~/tasks2"));
    expect(screen.queryByText(/^Saved: Tasks folder/)).toBeNull();
  });

  it("restart_now_with_a_running_agent_asks_first", async () => {
    const { native } = await settings(null, {
      native: daemonMachine({ readDaemonSettings: vi.fn().mockResolvedValue({ ...DAEMON_SETTINGS, tasksRoot: "~/tasks2" }) }),
      agents: [agent({ id: "w", status: "working" }) as Partial<Agent>],
    });
    await userEvent.click(await screen.findByRole("button", { name: "Restart now" }));
    expect(screen.getByRole("dialog", { name: "Restart the daemon?" })).toBeInTheDocument();
    await userEvent.click(button("Cancel"));
    expect(native!.saveAndRestart).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: "Restart the daemon?" })).toBeNull();
  });

  it("restart_now_is_disabled_with_the_restart_blocker", async () => {
    await settings(null, {
      native: daemonMachine({ readDaemonSettings: vi.fn().mockResolvedValue({ ...DAEMON_SETTINGS, tasksRoot: "~/tasks2" }) }),
      agents: [agent({ id: "q", status: "queued" }) as Partial<Agent>],
    });
    expect(await screen.findByRole("button", { name: "Restart now" })).toBeDisabled();
    expect(button("Restart now")).toHaveAttribute("title", "An agent is still starting; wait until it runs");
  });
});

describe("The wizard from the board", () => {
  it("restart_now_from_a_wizard_opened_on_the_board_asks_there", async () => {
    // A tool refusal on New task opens the wizard over the board; Restart now asks there, on Kanban.
    const native = daemonMachine();
    const tool = {
      kind: "not-installed" as const,
      tool: "gh" as const,
      fix: { install: "brew install gh", link: "https://cli.github.com", manager: "brew" },
    };
    const { FleetError } = await import("../api/client");
    const readChoices = vi.fn().mockResolvedValue({ ...(await native.readChoices()), product: "a", persona: "a" });
    await openApp("Kanban", null, {
      native: { ...native, readChoices },
      agents: [agent({ id: "w", status: "working" }) as Partial<Agent>],
      over: {
        recheckTools: vi.fn().mockResolvedValue(toolList()),
        spawn: vi.fn().mockRejectedValue(new FleetError("gh is missing", undefined, tool)),
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "New task" }));
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    await userEvent.click(within(screen.getByRole("dialog", { name: "New task" })).getByRole("button", { name: "Start task" }));
    await userEvent.click(await screen.findByRole("button", { name: "Open the setup wizard" }));
    expect(screen.queryByRole("dialog", { name: "New task" })).toBeNull();
    await changeTasksFolder("2");
    await userEvent.click(await within(wizard()!).findByRole("button", { name: "Restart now" }));
    expect(window.location.hash).toBe("#kanban");
    await userEvent.click(
      within(screen.getByRole("dialog", { name: "Restart the daemon?" })).getByRole("button", { name: "Restart anyway" }),
    );
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "restart" });
  });
});
