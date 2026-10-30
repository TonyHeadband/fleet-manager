import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ToolId } from "../api/generated/ToolId";
import type { ToolList } from "../api/generated/ToolList";
import type { Native } from "../native/native";
import { fakeClient, fakeNative, toolList } from "../test/fixtures";
import { Startup } from "./Startup";

// Ported from crates/fleet-gui/tests/shell.rs (launch and intro); the names are egui's.
const tools = (missing: Partial<Record<ToolId, boolean>> = {}) => toolList(missing);

function boot(
  over: {
    native?: Native | null;
    client?: Parameters<typeof fakeClient>[0];
    animate?: boolean;
  } = {},
) {
  const client = fakeClient({
    tools: vi.fn().mockResolvedValue(tools()),
    ...over.client,
  });
  const native = over.native === undefined ? fakeNative() : over.native;
  const connect = vi.fn(() => client);
  render(
    <Startup connect={connect} browserUrl="" native={native} animate={over.animate ?? false}>
      {() => <p>the board</p>}
    </Startup>,
  );
  return { client, native, connect };
}

describe("Startup, as egui's intro", () => {
  it("starting_shows_a_message_and_never_blocks_the_window", async () => {
    boot({
      native: fakeNative({ startDaemon: vi.fn(() => new Promise<never>(() => {})) }),
    });
    expect(await screen.findByText("Starting the background service…")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toBeInTheDocument();
    expect(screen.queryByText("the board")).toBeNull();
  });

  it("a_launch_failure_shows_the_reason_and_retry_runs_it_again", async () => {
    const startDaemon = vi
      .fn()
      .mockRejectedValueOnce("fleet-daemon was not found next to fleet-gui or on PATH")
      .mockResolvedValueOnce({ weStarted: true, url: "http://127.0.0.1:7878", local: true });
    const { native } = boot({ native: fakeNative({ startDaemon }) });
    expect(
      await screen.findByRole("heading", {
        name: "Fleet Manager could not start its daemon",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("fleet-daemon was not found next to fleet-gui or on PATH")).toBeInTheDocument();
    expect(await screen.findByText("State directory: /home/me/.local/share/fleet-manager")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(native!.startDaemon).toHaveBeenCalledTimes(2);
    expect(await screen.findByText("the board")).toBeInTheDocument();
  });

  it("a_started_daemon_leads_to_the_board", async () => {
    const { connect } = boot();
    expect(await screen.findByText("the board")).toBeInTheDocument();
    // On port 0 the address exists only once the daemon is up, so the client is made from the start's answer.
    expect(connect).toHaveBeenLastCalledWith("http://127.0.0.1:45123");
  });

  it("the_client_carries_the_started_daemons_token", async () => {
    const { client } = boot({ native: fakeNative({ daemonToken: vi.fn().mockResolvedValue("t0k") }) });
    expect(await screen.findByText("the board")).toBeInTheDocument();
    expect(client.token).toBe("t0k");
  });

  it("an_older_daemon_without_a_token_leaves_the_client_without_one", async () => {
    const { client } = boot({ native: fakeNative({ daemonToken: vi.fn().mockResolvedValue(null) }) });
    expect(await screen.findByText("the board")).toBeInTheDocument();
    expect(client.token).toBeNull();
  });

  it("all_tools_present_goes_straight_to_the_board_after_the_intro", async () => {
    // The clock moves only by hand: on a slow machine real time would step the rows on its own.
    vi.useFakeTimers();
    try {
      boot({ animate: true });
      // The start and the tools answer are promises: let them settle without moving the clock.
      for (let i = 0; i < 5; i++) await act(() => vi.advanceTimersByTimeAsync(0));
      expect(screen.getByText("Checking Git…")).toBeInTheDocument();
      await act(() => vi.advanceTimersByTimeAsync(150));
      expect(screen.getByText("Checking Claude Code…")).toBeInTheDocument();
      await act(() => vi.advanceTimersByTimeAsync(150));
      expect(screen.getByText("Checking GitHub CLI…")).toBeInTheDocument();
      await act(() => vi.advanceTimersByTimeAsync(150));
      expect(screen.getByText("the board")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("an_unanswered_tools_check_does_not_block_the_board", async () => {
    boot({
      client: { tools: vi.fn().mockRejectedValue(new Error("timed out")) },
    });
    expect(await screen.findByText("the board")).toBeInTheDocument();
  });

  it("the_daemon_is_started_once_even_when_react_runs_effects_twice", async () => {
    const native = fakeNative();
    render(
      <StrictMode>
        <Startup connect={() => fakeClient({ tools: vi.fn().mockResolvedValue(tools()) })} browserUrl="" native={native} animate={false}>
          {() => <p>the board</p>}
        </Startup>
      </StrictMode>,
    );
    expect(await screen.findByText("the board")).toBeInTheDocument();
    expect(native.startDaemon).toHaveBeenCalledTimes(1);
  });

  it("the_intro_logo_is_still_when_animations_are_off", async () => {
    boot({
      animate: false,
      native: fakeNative({ startDaemon: vi.fn(() => new Promise<never>(() => {})) }),
    });
    await screen.findByText("Starting the background service…");
    expect(document.querySelector(".boot-logo")).not.toHaveAttribute("data-spin");
  });

  it("the_browser_waits_for_the_daemon_on_the_boot_view", async () => {
    let answer!: () => void;
    const uiSchema = vi
      .fn()
      .mockRejectedValueOnce(new Error("down"))
      .mockImplementationOnce(() => new Promise<void>((r) => (answer = r)));
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      boot({ native: null, client: { uiSchema } });
      expect(await screen.findByText("Starting the background service…")).toBeInTheDocument();
      await act(() => vi.advanceTimersByTimeAsync(1000));
      expect(uiSchema).toHaveBeenCalledTimes(2);
      expect(screen.queryByText("the board")).toBeNull();
      await act(async () => answer());
      expect(await screen.findByText("the board")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("startup.css", () => {
  it("startup_css_uses_tokens_only", async () => {
    const { readFileSync } = await import("node:fs");
    const css = readFileSync("src/startup/startup.css", "utf8");
    expect(css.match(/(?<![\w.])\d+(\.\d+)?px\b/g)).toBeNull();
  });
});

// Ported from crates/fleet-gui/tests/{shell,requirements}.rs: the first run's folders step and Requirements.
describe("First run and Requirements, as egui's", () => {
  const firstRun = (over: Partial<Native> = {}) => fakeNative({ hasConfig: vi.fn().mockResolvedValue(false), ...over });

  it("missing_config_shows_first_run_with_the_default_root", async () => {
    const { native } = boot({ native: firstRun() });
    expect(await screen.findByRole("heading", { name: "Step 1 of 4 · Where Fleet Manager lives" })).toBeInTheDocument();
    expect(screen.getByText("Where should tasks live? Each task gets its own folder here.")).toBeInTheDocument();
    expect(screen.getByLabelText("Tasks folder")).toHaveValue("~/fleet-tasks");
    expect(screen.getByLabelText("Personas folder")).toHaveValue("~/.local/share/fleet-manager/personas");
    expect(
      screen.getByText("Each persona gets its own Claude and GitHub login here. Fleet Manager never reads the tokens."),
    ).toBeInTheDocument();
    expect(native!.startDaemon).not.toHaveBeenCalled();
  });

  it("continue_writes_the_config_and_creates_the_folder_then_starts", async () => {
    const { native } = boot({ native: firstRun() });
    await userEvent.click(await screen.findByRole("button", { name: "Next →" }));
    expect(native!.writeInitialConfig).toHaveBeenCalledWith("~/fleet-tasks", "~/.local/share/fleet-manager/personas");
    await waitFor(() => expect(native!.startDaemon).toHaveBeenCalled());
  });

  it("enter_in_a_field_is_next", async () => {
    const { native } = boot({ native: firstRun() });
    await userEvent.type(await screen.findByLabelText("Tasks folder"), "{Enter}");
    expect(native!.writeInitialConfig).toHaveBeenCalled();
  });

  it("an_unwritable_root_shows_the_error_and_stays", async () => {
    const { native } = boot({ native: firstRun({ writeInitialConfig: vi.fn().mockRejectedValue("Cannot create /x: Permission denied") }) });
    await userEvent.click(await screen.findByRole("button", { name: "Next →" }));
    expect(await screen.findByText("Cannot create /x: Permission denied")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Step 1 of 4 · Where Fleet Manager lives" })).toBeInTheDocument();
    expect(native!.startDaemon).not.toHaveBeenCalled();
  });

  it("an_empty_personas_folder_shows_the_error_and_stays", async () => {
    // The text is the bridge's (fleet-tauri's `save_first_run`, egui's); the screen shows it and keeps the step.
    const writeInitialConfig = vi.fn((_root: string, personas: string) =>
      personas.trim() ? Promise.resolve() : Promise.reject("Enter a folder for the personas"),
    );
    const { native } = boot({ native: firstRun({ writeInitialConfig }) });
    await userEvent.clear(await screen.findByLabelText("Personas folder"));
    await userEvent.click(screen.getByRole("button", { name: "Next →" }));
    expect(await screen.findByText("Enter a folder for the personas")).toBeInTheDocument();
    expect(native!.startDaemon).not.toHaveBeenCalled();
  });

  it("first_run_browse_fills_the_root", async () => {
    const { native } = boot({ native: firstRun({ pickFolder: vi.fn().mockResolvedValue("/home/me/work") }) });
    const [tasks] = await screen.findAllByRole("button", { name: "Browse…" });
    await userEvent.click(tasks!);
    expect(native!.pickFolder).toHaveBeenCalledWith("~/fleet-tasks");
    expect(screen.getByLabelText("Tasks folder")).toHaveValue("/home/me/work");
  });

  it("a_missing_tool_opens_requirements_with_its_fix", async () => {
    boot({ client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })) } });
    expect(await screen.findByRole("heading", { name: "Step 1 of 3 · What Fleet Manager needs" })).toBeInTheDocument();
    expect(screen.getByText("What the background service found on this machine.")).toBeInTheDocument();
    expect(screen.getByText("✓ Git 1.0")).toBeInTheDocument();
    expect(screen.getByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
    expect(screen.getByText("sudo dnf install gh")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "https://gh.invalid/install" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue without it" })).toBeInTheDocument();
    expect(screen.queryByText("the board")).toBeNull();
  });

  it("first_run_shows_requirements_even_when_all_is_found", async () => {
    boot({ native: firstRun() });
    await userEvent.click(await screen.findByRole("button", { name: "Next →" }));
    expect(await screen.findByRole("heading", { name: "Step 2 of 4 · What Fleet Manager needs" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument();
  });

  it("continue_without_a_tool_reaches_the_persona_step", async () => {
    boot({ client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })) } });
    await userEvent.click(await screen.findByRole("button", { name: "Continue without it" }));
    expect(await screen.findByText("the board")).toBeInTheDocument();
  });

  it("check_again_rechecks", async () => {
    const recheckTools = vi.fn().mockResolvedValue(tools());
    boot({ client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })), recheckTools } });
    await userEvent.click(await screen.findByRole("button", { name: "Check again" }));
    expect(recheckTools).toHaveBeenCalled();
    expect(await screen.findByText("✓ GitHub CLI (gh) 1.0")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument();
  });

  it("install_runs_the_command_then_rechecks", async () => {
    let closed!: () => void;
    const installTool = vi.fn(() => new Promise<void>((r) => (closed = r)));
    const recheckTools = vi.fn().mockResolvedValue(tools());
    const { native } = boot({
      native: fakeNative({ installTool }),
      client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })), recheckTools },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Install" }));
    expect(native!.installTool).toHaveBeenCalledWith("gh");
    expect(
      screen.getByText("Installing GitHub CLI (gh) in its own window; it is checked again when that window closes."),
    ).toBeInTheDocument();
    expect(recheckTools).not.toHaveBeenCalled();
    await act(async () => closed());
    expect(recheckTools).toHaveBeenCalled();
    expect(await screen.findByText("✓ GitHub CLI (gh) 1.0")).toBeInTheDocument();
  });

  it("while_installing_no_other_install_starts", async () => {
    boot({
      native: fakeNative({ installTool: vi.fn(() => new Promise<never>(() => {})) }),
      client: { tools: vi.fn().mockResolvedValue(tools({ claude: true, gh: true })) },
    });
    const installs = await screen.findAllByRole("button", { name: "Install" });
    expect(installs).toHaveLength(2);
    await userEvent.click(installs[1]!);
    expect(screen.queryAllByRole("button", { name: "Install" })).toHaveLength(0);
  });

  it("copy_and_the_link_go_through_the_bridge", async () => {
    const { native } = boot({ client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })) } });
    await userEvent.click(await screen.findByRole("button", { name: "Copy" }));
    expect(native!.copyText).toHaveBeenCalledWith("sudo dnf install gh");
    await userEvent.click(screen.getByRole("link", { name: "https://gh.invalid/install" }));
    expect(native!.openUrl).toHaveBeenCalledWith("https://gh.invalid/install");
  });

  it("the_browser_never_shows_the_folders_step", async () => {
    boot({ native: null });
    expect(await screen.findByText("the board")).toBeInTheDocument();
    expect(screen.queryByText(/Where Fleet Manager lives/)).toBeNull();
  });

  it("the_browser_shows_the_install_command_to_copy", async () => {
    boot({ native: null, client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })) } });
    expect(await screen.findByText("sudo dnf install gh")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("a_daemon_off_this_machine_offers_no_install", async () => {
    boot({
      native: fakeNative({ startDaemon: vi.fn().mockResolvedValue({ weStarted: false, url: "http://10.0.0.2:7878", local: false }) }),
      client: { tools: vi.fn().mockResolvedValue(tools({ gh: true })) },
    });
    expect(await screen.findByText("sudo dnf install gh")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });
});
