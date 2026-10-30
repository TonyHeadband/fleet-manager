import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fakeNative, toolList } from "../test/fixtures";
import { click, openApp, pending, type AppOptions } from "../management/harness";
import type { ToolList } from "../api/generated/ToolList";

// Ports of the Settings → Extras tests in crates/fleet-gui/tests/settings.rs.

/** The app on Settings → General, its tool checks answered by `recheckTools`. */
async function settings(recheckTools: ReturnType<typeof vi.fn>, opts: AppOptions = {}) {
  await openApp("Settings", null, { native: fakeNative(), ...opts, over: { recheckTools, ...opts.over } });
}

beforeEach(() => localStorage.clear());

describe("Settings → Extras", () => {
  it("extras_without_optional_tools_lists_the_required_ones_read_only", async () => {
    const recheckTools = vi.fn().mockResolvedValue(toolList({ claude: true, gh: true }));
    await settings(recheckTools);
    expect(recheckTools).not.toHaveBeenCalled();
    await click("Extras");
    expect(await screen.findByText("Claude Code is not installed")).toBeInTheDocument();
    expect(recheckTools).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("heading", { name: "Required tools" })).toBeInTheDocument();
    expect(screen.getByText("✓ Git 1.0")).toBeInTheDocument();
    expect(screen.getByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
    expect(screen.getByRole("button", { name: "Relaunch setup wizard" })).toBeInTheDocument();
    expect(
      screen.getByText("Checks the tools Fleet Manager needs and installs what is missing; the folders can be changed there too."),
    ).toBeInTheDocument();
    await click("General");
    await click("Extras");
    await waitFor(() => expect(recheckTools).toHaveBeenCalledTimes(2));
  });

  it("extras_install_an_optional_tool", async () => {
    const native = fakeNative();
    const recheckTools = vi.fn().mockResolvedValue(toolList({ claude: true, gh: true }, ["gh"]));
    await settings(recheckTools, { native, local: true });
    await click("Extras");
    expect(await screen.findByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Extras" })).toBeInTheDocument();
    expect(screen.queryByText("Claude Code is not installed")).toBeNull();
    expect(screen.queryByRole("button", { name: "Relaunch setup wizard" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Install" }));
    expect(native.installTool).toHaveBeenCalledWith("gh");
    await waitFor(() => expect(recheckTools).toHaveBeenCalledTimes(2));
  });

  it("extras_says_checking_until_the_first_answer", async () => {
    const held = pending<ToolList>();
    await settings(vi.fn().mockReturnValue(held.promise));
    await click("Extras");
    expect(screen.getByText("Checking the tools…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check again" })).toBeNull();
    await act(async () => held.resolve(toolList()));
    expect(screen.queryByText("Checking the tools…")).toBeNull();
    expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
  });

  it("a_failed_check_shows_its_error_and_keeps_the_list", async () => {
    const recheckTools = vi.fn().mockResolvedValueOnce(toolList()).mockRejectedValue(new Error("connection refused"));
    await settings(recheckTools);
    await click("Extras");
    expect(await screen.findByText("✓ Claude Code 1.0")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText("connection refused")).toBeInTheDocument();
    expect(screen.getByText("✓ Claude Code 1.0")).toBeInTheDocument();
  });

  it("a_second_check_again_keeps_only_the_latest_answer", async () => {
    const first = pending<ToolList>();
    const second = pending<ToolList>();
    const recheckTools = vi.fn().mockResolvedValueOnce(toolList()).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    await settings(recheckTools);
    await click("Extras");
    await screen.findByText("✓ GitHub CLI (gh) 1.0");
    await userEvent.click(screen.getByRole("button", { name: "Check again" }));
    await userEvent.click(screen.getByRole("button", { name: "Check again" }));
    await act(async () => second.resolve(toolList({ gh: true })));
    expect(screen.getByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
    await act(async () => first.resolve(toolList()));
    expect(screen.getByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
  });

  it("an_answer_adopts_its_path_when_the_daemon_is_local", async () => {
    const native = fakeNative();
    await settings(vi.fn().mockResolvedValue({ ...toolList(), path: "/usr/local/bin:/usr/bin" }), { native, local: true });
    await click("Extras");
    await waitFor(() => expect(native.adoptPath).toHaveBeenCalledWith());
  });
});
