import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type { FleetClient } from "./api/client";
import { FakeEventSource, agent, agentIn, schema } from "./test/fixtures";

const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;

function fakeClient(over: Partial<Record<keyof FleetClient, unknown>> = {}): FleetClient {
  return {
    baseUrl: "",
    uiSchema: vi.fn().mockResolvedValue(schema),
    repos: vi.fn().mockResolvedValue(["core", "portal"]),
    spawn: vi.fn().mockResolvedValue(agent()),
    run: vi.fn().mockResolvedValue(undefined),
    diff: vi.fn().mockResolvedValue({ workspaces: [] }),
    ...over,
  } as unknown as FleetClient;
}

beforeEach(() => FakeEventSource.reset());

describe("App", () => {
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
    act(() => es.emit("snapshot", { seq: 1, agents: [agentIn("core", { id: "a", task: "core task" }), agentIn("portal", { id: "b", task: "portal task" }), agent({ id: "c", task: "both task", workspaces: [{ repo: "core", branch: "b", worktree: "/w", base: "x", has_changes: false }, { repo: "portal", branch: "b", worktree: "/w2", base: "x", has_changes: false }] })] }, "1"));
    await userEvent.click(screen.getByRole("button", { name: "portal" }));
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
    expect(await screen.findByRole("alert")).toHaveTextContent("agent is not running");
  });

  it("opens a read-only preview for the clicked card", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    expect(FakeEventSource.for("/api/agents/a/terminal")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Diff" })).toBeInTheDocument();
  });
});
