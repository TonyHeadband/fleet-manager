import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { FakeEventSource, agent, agentIn, factory, fakeClient, persona, product } from "./test/fixtures";

beforeEach(() => {
  FakeEventSource.reset();
  window.location.hash = "";
});

describe("App", () => {
  it("New_task_says_why_while_the_daemon_is_down", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    localStorage.setItem("fleet.product", "p1");
    const personas = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue({ personas: [persona("me")] });
    const products = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue({ products: [product("p1")] });
    render(<App client={fakeClient({ personas, products })} eventSource={factory} nowSeconds={() => 1300} />);
    const newTask = await screen.findByRole("button", { name: "New task" });
    await waitFor(() => expect(newTask).toHaveAttribute("title", "Could not load the personas; retrying"));
    expect(newTask).toBeDisabled();
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(newTask).toBeEnabled();
    vi.useRealTimers();
    localStorage.clear();
  });

  it("App_moves_a_card_when_a_status_event_arrives", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const es = FakeEventSource.for("/api/events");
    act(() => es.emit("snapshot", { seq: 1, agents: [agent({ id: "a1", status: "working" })] }, "1"));
    expect(within(screen.getByRole("region", { name: "Working" })).getByText("fix the tests")).toBeInTheDocument();
    act(() =>
      es.emit("event", { seq: 2, kind: { type: "status_changed", id: "a1", status: "needs_you", activity: "wants to run Bash", since: 1200, has_changes: false, paused: false, blocked_reason: "permission" } }, "2"),
    );
    expect(within(screen.getByRole("region", { name: "Needs you" })).getByText("fix the tests")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Working" })).queryByText("fix the tests")).not.toBeInTheDocument();
  });

  it("RepoFilter_shows_agents_touching_any_selected_repo", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const es = FakeEventSource.for("/api/events");
    act(() => es.emit("snapshot", { seq: 1, agents: [agentIn("core", { id: "a", goal: "core task" }), agentIn("portal", { id: "b", goal: "portal task" }), agent({ id: "c", goal: "both task", workspaces: [{ repo: "core", branch: "b", worktree: "/w", base: "x", has_changes: false, owned: false }, { repo: "portal", branch: "b", worktree: "/w2", base: "x", has_changes: false, owned: false }] })] }, "1"));
    await userEvent.click(within(screen.getByRole("group", { name: "Repo filter" })).getByRole("button", { name: /^portal/ }));
    expect(screen.queryByText("core task")).not.toBeInTheDocument();
    expect(screen.getByText("portal task")).toBeInTheDocument();
    expect(screen.getByText("both task")).toBeInTheDocument();
  });

  it("runs a card action through the client and shows a failure", async () => {
    const run = vi.fn().mockRejectedValue(new Error("agent is not running"));
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(run).toHaveBeenCalledWith(expect.objectContaining({ id: "pause" }), expect.objectContaining({ id: "a" }), undefined);
    // A failure goes to the footer's message line, as in Lite.
    expect(await within(screen.getByRole("contentinfo")).findByText("agent is not running")).toBeInTheDocument();
  });

  it("opens a read-only preview for the clicked card", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    expect(FakeEventSource.for("/api/agents/a/terminal")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Diff" })).toBeInTheDocument();
  });

  it("App_switches_between_the_board_and_the_forms_page", async () => {
    const products = vi.fn().mockResolvedValue({ products: [{ id: "pie", name: "Pie", default_persona: null, repos: [], releases: [], tasks: 0, overrides: [] }] });
    const agentConfig = vi.fn().mockResolvedValue({ instructions: "", settings: { allow: [], deny: [], env: {} } });
    render(<App client={fakeClient({ products, agentConfig })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    // The debug page is reached by its address; the drawer lists only Lite's views.
    window.location.hash = "#forms";
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(await screen.findByRole("heading", { name: "New release" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Working" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Kanban" }));
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(await screen.findByRole("region", { name: "Working" })).toBeInTheDocument();
  });
});
