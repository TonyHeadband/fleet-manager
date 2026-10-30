import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { agent, closingNative } from "../test/fixtures";
import { CloseDialog } from "./CloseDialog";
import { dialogText } from "./close";

// Ported from crates/fleet-gui/tests/close_dialog.rs and the close flow's tests in shell.rs.
const agents = [agent({ id: "a", status: "working" }), agent({ id: "b", status: "needs_you" }), agent({ id: "c", status: "stopped" })];

function open(list = agents) {
  const { native, emit } = closingNative();
  const view = render(<CloseDialog native={native} agents={list} />);
  act(() => emit({ phase: "asking" }));
  return { native, emit, view };
}

describe("Close, as egui's", () => {
  it("dialog_text_is_fleet_client_s", () => {
    expect(dialogText({ running: 0, working: 0, needs_you: 0 })).toBe(
      "No agents are running, so stopping the daemon stops nothing. Cards, branches and worktrees are kept.",
    );
    expect(dialogText({ running: 1, working: 0, needs_you: 0 })).toBe(
      "1 agent is running. It will stop now and resume automatically next time the daemon starts. Cards, branches and worktrees are kept.",
    );
    expect(dialogText({ running: 3, working: 1, needs_you: 2 })).toBe(
      "3 agents are running. They will stop now and resume automatically next time the daemon starts. Now: 1 working, 2 waiting for you. Cards, branches and worktrees are kept.",
    );
  });

  it("dialog_shows_the_numbers_both_choices_and_a_remember_box_with_keep_running_focused", () => {
    open();
    expect(screen.getByRole("heading", { name: "Closing Fleet Manager" })).toBeInTheDocument();
    expect(screen.getByText(dialogText({ running: 2, working: 1, needs_you: 1 }))).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Remember my choice" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Keep the daemon running" })).toHaveFocus();
    expect(screen.getByRole("button", { name: "Stop the daemon too" })).toBeInTheDocument();
    expect(screen.getByText("You can change this later in Settings.")).toBeInTheDocument();
  });

  it("keep_running_and_stop_report_their_choice", async () => {
    const { native } = open();
    await userEvent.click(screen.getByRole("button", { name: "Stop the daemon too" }));
    expect(native.answerClose).toHaveBeenCalledWith("stop", false);
    expect(screen.getByText("Stopping the daemon…")).toBeInTheDocument();
    const again = open();
    await userEvent.click(screen.getAllByRole("button", { name: "Keep the daemon running" }).at(-1)!);
    expect(again.native.answerClose).toHaveBeenCalledWith("keep", false);
  });

  it("cancel_and_escape_leave_the_window_open", async () => {
    const { native, emit } = open();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(native.answerClose).toHaveBeenCalledWith("cancel", false);
    expect(screen.queryByRole("heading", { name: "Closing Fleet Manager" })).toBeNull();
    act(() => emit({ phase: "asking" }));
    await userEvent.keyboard("{Escape}");
    expect(native.answerClose).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("heading", { name: "Closing Fleet Manager" })).toBeNull();
  });

  it("remember_is_reported_with_the_choice", async () => {
    const { native } = open();
    await userEvent.click(screen.getByRole("checkbox", { name: "Remember my choice" }));
    await userEvent.click(screen.getByRole("button", { name: "Keep the daemon running" }));
    expect(native.answerClose).toHaveBeenCalledWith("keep", true);
  });

  it("the_close_dialog_counts_follow_the_agents_while_it_is_open", () => {
    const { native, view } = open();
    view.rerender(<CloseDialog native={native} agents={[agent({ status: "working" })]} />);
    expect(screen.getByText(dialogText({ running: 1, working: 1, needs_you: 0 }))).toBeInTheDocument();
  });

  it("a_failed_stop_is_shown_and_the_user_can_close_anyway", async () => {
    const { native, emit } = open();
    act(() => emit({ phase: "stop-failed", message: "the daemon did not answer" }));
    expect(screen.getByRole("heading", { name: "The daemon did not stop" })).toBeInTheDocument();
    expect(screen.getByText("the daemon did not answer")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Close anyway" }));
    expect(native.answerClose).toHaveBeenCalledWith("close-anyway", false);
  });

  it("closing_again_while_a_stop_is_running_does_not_start_a_second_stop", async () => {
    const { native, emit } = open();
    await userEvent.click(screen.getByRole("button", { name: "Stop the daemon too" }));
    // The Rust side ignores a second request while stopping; the window keeps saying so.
    act(() => emit({ phase: "stopping" }));
    expect(screen.getByText("Stopping the daemon…")).toBeInTheDocument();
    expect(native.answerClose).toHaveBeenCalledTimes(1);
  });

  it("a_second_close_request_while_the_dialog_is_open_keeps_it_as_it_is", async () => {
    const { emit } = open();
    await userEvent.click(screen.getByRole("checkbox", { name: "Remember my choice" }));
    act(() => emit({ phase: "asking" }));
    expect(screen.getByRole("checkbox", { name: "Remember my choice" })).toBeChecked();
  });

  it("the_browser_has_no_close_flow", () => {
    render(<CloseDialog native={null} agents={agents} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
