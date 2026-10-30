import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ToolList } from "../api/generated/ToolList";
import type { Native } from "../native/native";
import { fakeClient } from "../test/fixtures";
import { Startup } from "./Startup";

// Ported from crates/fleet-gui/tests/shell.rs (launch and intro); the names are egui's.
const tools = (missing: Partial<Record<string, boolean>> = {}): ToolList => ({
  tools: (["git", "claude", "gh"] as const).map((tool) => ({
    tool,
    required: true,
    version: missing[tool] ? null : "1.0",
    error: missing[tool]
      ? {
          kind: "not-installed",
          tool,
          fix: { install: null, link: "https://x.invalid", manager: null },
        }
      : null,
  })),
  checked_at: 0n,
  path: null,
});

export function fakeNative(over: Partial<Native> = {}): Native {
  return {
    version: vi.fn().mockResolvedValue("0.2.0"),
    hasConfig: vi.fn().mockResolvedValue(true),
    startDaemon: vi.fn().mockResolvedValue({ weStarted: true, url: "http://127.0.0.1:45123" }),
    stateDir: vi.fn().mockResolvedValue("/home/me/.local/share/fleet-manager"),
    ...over,
  } as Native;
}

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
      .mockResolvedValueOnce({ weStarted: true, url: "http://127.0.0.1:7878" });
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

  it("all_tools_present_goes_straight_to_the_board_after_the_intro", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      boot({ animate: true });
      expect(await screen.findByText("Checking Git…")).toBeInTheDocument();
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
