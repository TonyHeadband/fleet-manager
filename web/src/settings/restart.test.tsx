import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agent, fakeNative, RESTARTED } from "../test/fixtures";
import { openApp, pending, type AppOptions } from "../management/harness";
import { confirmText, restartBlocker } from "./restart";
import type { Agent } from "../api/generated/Agent";
import { DEFAULT_CHOICES } from "../native/native";

// Ports of the restart tests in crates/fleet-gui/tests/settings.rs and shell.rs, through the Daemon tab's
// Restart daemon; the Saves' own ones are in daemon.test.tsx.

const footer = () => screen.getByRole("contentinfo");
const restartButton = () => screen.getByRole("button", { name: "Restart daemon" });
const dialog = () => screen.queryByRole("dialog", { name: "Restart the daemon?" });
const working = (id: string, status: Agent["status"] = "working") => agent({ id, status }) as Partial<Agent>;
const requesting = () => agent({ id: "r", status: "needs_you", repo_request: { repo: "portal", why: "login" } }) as Partial<Agent>;

/** The app on Settings → Daemon, in the Tauri app. */
function daemonTab(opts: AppOptions = {}) {
  return openApp("Settings", "Daemon", { native: fakeNative(), ...opts });
}

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

describe("The restart's words and blockers", () => {
  it("restart_blocker_names_the_first_reason_in_egui_s_order", () => {
    const queued = [agent({ status: "queued" })];
    expect(restartBlocker({ restarting: true, agents: queued, spawning: true, deciding: true })).toBe("The daemon is restarting");
    expect(restartBlocker({ restarting: false, agents: queued, spawning: true, deciding: true })).toBe(
      "An agent is still starting; wait until it runs",
    );
    expect(restartBlocker({ restarting: false, agents: [], spawning: true, deciding: true })).toBe("A task is being started");
    expect(restartBlocker({ restarting: false, agents: [], spawning: false, deciding: true })).toBe("A repo request is being answered");
    expect(restartBlocker({ restarting: false, agents: [], spawning: false, deciding: false })).toBeNull();
  });

  it("the_confirmation_adds_the_repo_request_sentence_for_a_save_or_a_restart", () => {
    const agents = [agent({ status: "needs_you", repo_request: { repo: "portal", why: "login" } })];
    expect(confirmText(agents, "tasksRoot")).toContain(
      " An agent is waiting for an answer about a repo. Saving restarts the daemon, which ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
    );
    expect(confirmText(agents, "restart")).toContain(
      " An agent is waiting for an answer about a repo. Restarting the daemon ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
    );
    expect(confirmText([agent({ status: "stopped" })], "restart")).toBe(
      "No agents are running, so stopping the daemon stops nothing. Cards, branches and worktrees are kept.",
    );
  });
});

describe("Restart daemon", () => {
  it("daemon_tab_shows_paths", async () => {
    const daemonInfo = vi.fn().mockResolvedValue({
      address: "127.0.0.1:7878",
      startedBy: "already running",
      stateDir: "/s",
      configFile: "/c/config.toml",
      pid: "4242",
    });
    await daemonTab({ native: fakeNative({ daemonInfo }) });
    for (const text of [
      "Address",
      "Started by",
      "State directory",
      "Config file",
      "PID",
      "127.0.0.1:7878",
      "already running",
      "4242",
      "/s",
      "/c/config.toml",
    ])
      expect(await screen.findByText(text)).toBeInTheDocument();
    // The pidfile is read again while the tab shows.
    daemonInfo.mockResolvedValue({
      address: "127.0.0.1:7878",
      startedBy: "already running",
      stateDir: "/s",
      configFile: "/c/config.toml",
      pid: null,
    });
    await waitFor(() => expect(screen.queryByText("PID")).toBeNull(), { timeout: 2000 });
    expect(restartButton()).toBeEnabled();
  });

  it("restart_daemon_with_running_agents_asks_first", async () => {
    const native = fakeNative();
    await daemonTab({ native, agents: [working("w"), working("v", "review"), working("s", "stopped")] });
    await userEvent.click(restartButton());
    expect(
      within(dialog()!).getByText(
        /2 agents are running\. They will stop now and resume automatically next time the daemon starts\. Now: 1 working\./,
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(dialog()).toBeNull();
    expect(native.saveAndRestart).not.toHaveBeenCalled();
    await userEvent.click(restartButton());
    await userEvent.click(screen.getByRole("button", { name: "Restart anyway" }));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "restart" });
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
  });

  it("restart_daemon_with_a_pending_request_asks_first", async () => {
    await daemonTab({ agents: [requesting()] });
    await userEvent.click(restartButton());
    expect(within(dialog()!).getByText(/Restarting the daemon ends that request/)).toBeInTheDocument();
  });

  it("with_no_snapshot_yet_a_restart_asks_first", async () => {
    // The stream never sent a snapshot: nobody knows what runs, so it asks, with the no-agents text.
    const native = fakeNative();
    await openApp("Settings", "Daemon", { native, snapshot: false });
    await userEvent.click(restartButton());
    expect(
      within(dialog()!).getByText("No agents are running, so stopping the daemon stops nothing. Cards, branches and worktrees are kept."),
    ).toBeInTheDocument();
  });

  it("failed_restart_offers_restart_daemon", async () => {
    const saveAndRestart = vi.fn().mockRejectedValueOnce(new Error("fleet-daemon exited at startup")).mockResolvedValue(RESTARTED);
    await daemonTab({ native: fakeNative({ saveAndRestart }) });
    await userEvent.click(restartButton());
    expect(await within(footer()).findByText("fleet-daemon exited at startup")).toBeInTheDocument();
    await waitFor(() => expect(restartButton()).toBeEnabled());
    await userEvent.click(restartButton());
    expect(saveAndRestart).toHaveBeenCalledTimes(2);
  });

  it("a_restart_without_a_pidfile_says_so", async () => {
    await daemonTab({
      native: fakeNative({ saveAndRestart: vi.fn().mockResolvedValue({ ...RESTARTED, outcome: "alreadyRunning", weStarted: false }) }),
    });
    await userEvent.click(restartButton());
    expect(
      await within(footer()).findByText(
        "Could not restart the daemon: no daemon.pid: this daemon was not started by a version that writes one",
      ),
    ).toBeInTheDocument();
  });

  it("a_panicking_restart_thread_still_ends_the_restart", async () => {
    await daemonTab({ native: fakeNative({ saveAndRestart: vi.fn().mockRejectedValue(new Error("restart thread panicked")) }) });
    await userEvent.click(restartButton());
    expect(await within(footer()).findByText("restart thread panicked")).toBeInTheDocument();
    await waitFor(() => expect(restartButton()).toBeEnabled());
  });

  it("a_second_click_while_restarting_does_nothing", async () => {
    const held = pending<typeof RESTARTED>();
    const native = fakeNative({ saveAndRestart: vi.fn().mockReturnValue(held.promise) });
    await daemonTab({ native });
    await userEvent.click(restartButton());
    expect(screen.getByText("Restarting the daemon…")).toBeInTheDocument();
    expect(restartButton()).toBeDisabled();
    expect(restartButton()).toHaveAttribute("title", "The daemon is restarting");
    await userEvent.click(restartButton());
    expect(native.saveAndRestart).toHaveBeenCalledTimes(1);
    await act(async () => held.resolve(RESTARTED));
    expect(screen.queryByText("Restarting the daemon…")).toBeNull();
  });

  it("new_task_is_blocked_while_the_daemon_restarts", async () => {
    const held = pending<typeof RESTARTED>();
    const readChoices = vi.fn().mockResolvedValue({ ...DEFAULT_CHOICES, product: "a" });
    await daemonTab({ native: fakeNative({ readChoices, saveAndRestart: vi.fn().mockReturnValue(held.promise) }) });
    const newTask = screen.getByRole("button", { name: "New task" });
    await waitFor(() => expect(newTask).toBeEnabled());
    await userEvent.click(restartButton());
    expect(newTask).toBeDisabled();
    expect(newTask).toHaveAttribute("title", "The daemon is restarting");
    await act(async () => held.resolve(RESTARTED));
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
    expect(newTask).toBeEnabled();
  });

  it("a_call_in_flight_blocks_save_and_restart_daemon_with_its_reason", async () => {
    await daemonTab({ agents: [agent({ id: "q", status: "queued" }) as Partial<Agent>] });
    expect(restartButton()).toBeDisabled();
    expect(restartButton()).toHaveAttribute("title", "An agent is still starting; wait until it runs");
  });

  it("a_restart_makes_the_window_own_the_daemon", async () => {
    const daemonInfo = vi
      .fn()
      .mockResolvedValue({ address: "a", startedBy: "already running", stateDir: "/s", configFile: "/c", pid: null });
    const saveAndRestart = vi.fn(async () => {
      daemonInfo.mockResolvedValue({ address: "a", startedBy: "this window", stateDir: "/s", configFile: "/c", pid: "7" });
      return RESTARTED;
    });
    await daemonTab({ native: fakeNative({ daemonInfo, saveAndRestart }) });
    expect(await screen.findByText("already running")).toBeInTheDocument();
    await userEvent.click(restartButton());
    expect(await screen.findByText("this window")).toBeInTheDocument();
  });

  it("a_restart_on_another_address_reloads_the_page", async () => {
    const native = fakeNative({ saveAndRestart: vi.fn().mockResolvedValue({ ...RESTARTED, url: "http://127.0.0.1:9999" }) });
    await daemonTab({ native });
    // Spied once the page is open: the route reads the real location's hash.
    const reload = vi.fn();
    const real = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({
      href: real.href,
      origin: real.origin,
      hash: real.hash,
      reload,
    } as unknown as Location);
    await userEvent.click(restartButton());
    await waitFor(() => expect(reload).toHaveBeenCalled());
    expect(native.setCloseArmed).toHaveBeenCalledWith(false);
  });

  it("daemon_restarted_is_said_after_the_reload", async () => {
    // The page that reloads cannot show it; the next one does (#418).
    const native = fakeNative({ saveAndRestart: vi.fn().mockResolvedValue({ ...RESTARTED, url: "http://127.0.0.1:9999" }) });
    await daemonTab({ native });
    const real = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({
      href: real.href,
      origin: real.origin,
      hash: real.hash,
      reload: vi.fn(),
    } as unknown as Location);
    await userEvent.click(restartButton());
    await waitFor(() => expect(native.setCloseArmed).toHaveBeenCalledWith(false));
    vi.restoreAllMocks();
    cleanup();
    await openApp("Kanban", null, { native: fakeNative() });
    expect(await within(footer()).findByText("Daemon restarted")).toBeInTheDocument();
  });

  it("a_restart_on_the_same_address_loads_the_lists_again", async () => {
    const client = await daemonTab();
    const before = vi.mocked(client.uiSchema).mock.calls.length;
    await userEvent.click(restartButton());
    await waitFor(() => expect(vi.mocked(client.uiSchema).mock.calls.length).toBeGreaterThan(before));
    expect(client.products).toHaveBeenCalledTimes(2);
  });

  it("in_a_browser_the_daemon_tab_says_only_in_the_app", async () => {
    await openApp("Settings", "Daemon", {});
    expect(screen.getByText("Only in the app")).toBeInTheDocument();
    expect(restartButton()).toBeDisabled();
    expect(restartButton()).toHaveAttribute("title", "Only in the app");
  });
});

describe("The version offer's Restart", () => {
  it("a_queued_agent_still_lets_the_offer_ask", async () => {
    // egui's `ask_restart` does not look at the blocker: the dialog opens, and its Go waits for the agent (#418).
    const native = fakeNative();
    await openApp("Kanban", null, {
      native,
      agents: [agent({ id: "q", status: "queued" })],
      over: { version: vi.fn().mockResolvedValue({ version: "0.0.1" }) },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Restart" }));
    expect(dialog()).not.toBeNull();
  });

  it("restarting_the_old_daemon_confirms_when_agents_run", async () => {
    const native = fakeNative();
    await openApp("Kanban", null, {
      native,
      agents: [working("w")],
      over: { version: vi.fn().mockResolvedValue({ version: "0.0.1" }) },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Restart" }));
    const d = dialog()!;
    expect(within(d).getByText(/running/)).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "Restart anyway" })).toBeInTheDocument();
    expect(native.saveAndRestart).not.toHaveBeenCalled();
  });
});
