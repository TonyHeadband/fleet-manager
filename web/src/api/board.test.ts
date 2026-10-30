import { describe, expect, it } from "vitest";
import { boardReducer } from "./board";
import type { Snapshot } from "./generated/Snapshot";
import type { Event } from "./generated/Event";
import { agent, workspace } from "../test/fixtures";

const base: Snapshot = { seq: 5, agents: [agent({ id: "a1" }), agent({ id: "a2", status: "queued" })] };

describe("boardReducer", () => {
  it("boardReducer_applies_title_changed", () => {
    const s = boardReducer(base, { seq: 6, kind: { type: "title_changed", id: "a1", title: "Flaky tests" } });
    expect(s.agents[0]?.title).toBe("Flaky tests");
    expect(s.agents[1]).toEqual(base.agents[1]);
  });
  it("boardReducer_applies_status_changed_and_removed", () => {
    const changed: Event = {
      seq: 6,
      kind: { type: "status_changed", id: "a1", status: "needs_you", activity: "wants to run Bash", since: 2000, has_changes: true, paused: false, blocked_reason: "permission" },
    };
    const s1 = boardReducer(base, changed);
    expect(s1.seq).toBe(6);
    expect(s1.agents[0]).toMatchObject({ id: "a1", status: "needs_you", activity: "wants to run Bash", status_since: 2000, has_changes: true, blocked_reason: "permission" });
    expect(s1.agents[1]).toEqual(base.agents[1]);
    const s2 = boardReducer(s1, { seq: 7, kind: { type: "agent_removed", id: "a2" } });
    expect(s2.agents.map((a) => a.id)).toEqual(["a1"]);
  });

  it("boardReducer_applies_workspaces_changed", () => {
    const withTwo = { seq: 5, agents: [agent({ id: "a1", workspaces: [workspace("core"), workspace("portal")], has_changes: false })] };
    const flipped = boardReducer(withTwo, { seq: 6, kind: { type: "workspaces_changed", id: "a1", workspaces: [workspace("core"), workspace("portal", true)] } });
    expect(flipped.seq).toBe(6);
    expect(flipped.agents[0]!.workspaces.map((w) => w.has_changes)).toEqual([false, true]);
    expect(flipped.agents[0]!.has_changes).toBe(true);
    const cleared = boardReducer(flipped, { seq: 7, kind: { type: "workspaces_changed", id: "a1", workspaces: [workspace("core"), workspace("portal")] } });
    expect(cleared.agents[0]!.has_changes).toBe(false);
  });

  it("appends created agents once", () => {
    const created: Event = { seq: 6, kind: { type: "agent_created", agent: agent({ id: "a3" }) } };
    const s1 = boardReducer(base, created);
    expect(s1.agents.map((a) => a.id)).toEqual(["a1", "a2", "a3"]);
    expect(boardReducer(s1, created).agents).toHaveLength(3);
  });

  it("ignores events at or below the current seq (replays after reconnect)", () => {
    const stale: Event = { seq: 5, kind: { type: "agent_removed", id: "a1" } };
    expect(boardReducer(base, stale)).toBe(base);
  });

  it("a_status_event_carries_the_stop_cause_and_the_repo_request", () => {
    const request = { repo: "core", reason: "needs it" };
    const ended = boardReducer(base, {
      seq: 6,
      kind: { type: "status_changed", id: "a1", status: "stopped", activity: "", since: 2000, has_changes: false, paused: false, blocked_reason: null, stop_cause: "crashed", repo_request: request as never, disk_bytes: 42 },
    });
    expect(ended.agents[0]).toMatchObject({ stop_cause: "crashed", repo_request: request, disk_bytes: 42 });
    // As Lite's board.rs: each event sets them, so one without them clears them.
    const again = boardReducer(ended, { seq: 7, kind: { type: "status_changed", id: "a1", status: "working", activity: "", since: 2100, has_changes: false, paused: false, blocked_reason: null } });
    expect(again.agents[0]).toMatchObject({ stop_cause: null, repo_request: null, disk_bytes: null });
  });

  it("advances seq but changes nothing for an unknown agent", () => {
    const s = boardReducer(base, { seq: 6, kind: { type: "status_changed", id: "ghost", status: "review", activity: "", since: 1, has_changes: false, paused: false, blocked_reason: null } });
    expect(s.seq).toBe(6);
    expect(s.agents).toEqual(base.agents);
  });
});
