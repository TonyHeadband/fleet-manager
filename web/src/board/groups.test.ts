import { describe, expect, it } from "vitest";
import type { Agent } from "../api/generated/Agent";
import type { AgentKind } from "../api/generated/AgentKind";
import type { Status } from "../api/generated/Status";
import { agent, persona } from "../test/fixtures";
import { canRestart, fleetName, groups, leadDown, rollup, stuck, taskId } from "./groups";

// Ported from crates/fleet-client/tests/schema.rs and crates/fleet-api/tests/contract.rs; the names are Lite's.
const member = (id: string, kind: AgentKind, task: string | null, status: Status, since: number, over: Partial<Agent> = {}) =>
  agent({ id, kind, task, status, status_since: since, ...over });
const ids = (agents: Agent[]) => agents.map((a) => a.id);

describe("grouping", () => {
  it("rollup_order", () => {
    expect(rollup(["review", "needs_you", "working"])).toBe("needs_you");
    expect(rollup(["review", "working"])).toBe("working");
    expect(rollup(["talking", "queued"])).toBe("working");
    expect(rollup(["queued", "review"])).toBe("queued");
    expect(rollup(["review", "stopped"])).toBe("review");
    expect(rollup([])).toBe("stopped");
  });

  it("task_id_prefers_the_task_field", () => {
    expect(taskId(agent({ id: "a1" }))).toBe("a1");
    expect(taskId(agent({ id: "a1", task: "p1" }))).toBe("p1");
  });

  it("groups_and_rollup", () => {
    // A fleet agent that needs you sends the whole task to Needs you.
    let g = groups([member("o", "orchestrator", null, "working", 10), member("f1", "specialist", "o", "working", 20), member("f2", "specialist", "o", "needs_you", 30)]);
    expect(g).toHaveLength(1);
    expect(g[0]!.lead.id).toBe("o");
    expect(g[0]!.status).toBe("needs_you");
    expect(ids(g[0]!.fleet), "the stuck agent first").toEqual(["f2", "f1"]);
    expect(stuck(g[0]!)?.id).toBe("f2");

    // Ended fleet agents do not count: Review once every live agent is done.
    g = groups([
      member("o", "orchestrator", null, "review", 10),
      member("f1", "specialist", "o", "stopped", 20),
      member("f2", "specialist", "o", "stopped", 30, { stop_cause: "finished" }),
    ]);
    expect(g[0]!.status).toBe("review");
    expect(leadDown(g[0]!)).toBe(false);

    // A restarted orchestrator leads; the old one is an ended row of the fleet.
    g = groups([member("o", "orchestrator", null, "stopped", 10), member("f1", "specialist", "o", "working", 20), member("o2", "orchestrator", "o", "queued", 40)]);
    expect(g).toHaveLength(1);
    expect(g[0]!.lead.id).toBe("o2");
    expect(ids(g[0]!.fleet)).toEqual(["o", "f1"]);
    expect(g[0]!.status).toBe("working");

    // A crashed orchestrator with live fleet goes to Needs you.
    g = groups([member("o", "orchestrator", null, "stopped", 10, { stop_cause: "crashed" }), member("f1", "specialist", "o", "working", 20)]);
    expect(g[0]!.status).toBe("needs_you");
    expect(leadDown(g[0]!)).toBe(true);
    expect(stuck(g[0]!), "no fleet agent is the reason").toBeNull();

    // Every agent stopped: the task is stopped, not waiting on you.
    g = groups([member("o", "orchestrator", null, "stopped", 10), member("f1", "specialist", "o", "stopped", 20)]);
    expect(g[0]!.status).toBe("stopped");

    // A solo agent is its own group, with no fleet; Talking counts as Working.
    g = groups([member("s", "solo", null, "talking", 5), member("o", "orchestrator", null, "queued", 10)]);
    expect(g).toHaveLength(2);
    expect(g[0]!.lead.id).toBe("s");
    expect(g[0]!.fleet).toEqual([]);
    expect(g[0]!.status).toBe("working");
    expect(g[1]!.lead.id).toBe("o");
    expect(g[1]!.fleet).toEqual([]);
  });

  it("an_ended_orchestrator_asks_for_a_restart", () => {
    // Crashed or exited with no live fleet: the task is not known to be done, so it needs you.
    for (const cause of ["crashed", "exited"] as const) {
      const g = groups([member("o", "orchestrator", null, "stopped", 10, { stop_cause: cause }), member("f1", "specialist", "o", "stopped", 20, { stop_cause: "finished" })]);
      expect(g[0]!.status, cause).toBe("needs_you");
      expect(leadDown(g[0]!), cause).toBe(true);
      expect(canRestart(g[0]!), cause).toBe(true);
    }

    // Stopped by Stop task: the task is stopped, but it can still be restarted.
    let g = groups([member("o", "orchestrator", null, "stopped", 10, { stop_cause: "stopped" }), member("f1", "specialist", "o", "stopped", 20, { stop_cause: "stopped" })]);
    expect(g[0]!.status).toBe("stopped");
    expect(leadDown(g[0]!)).toBe(false);
    expect(canRestart(g[0]!)).toBe(true);

    // A running orchestrator and a crashed solo agent have nothing to restart.
    g = groups([member("s", "solo", null, "stopped", 5, { stop_cause: "crashed" }), member("o", "orchestrator", null, "working", 10)]);
    expect(g[0]!.status).toBe("stopped");
    expect(leadDown(g[0]!) || canRestart(g[0]!)).toBe(false);
    expect(leadDown(g[1]!) || canRestart(g[1]!)).toBe(false);
  });

  it("an_agent_without_a_kind_is_solo", () => {
    const old = agent({ id: "a1" });
    delete old.kind;
    expect(groups([old])[0]).toMatchObject({ lead: { id: "a1" }, fleet: [], status: "working" });
  });

  it("a_fleet_without_its_orchestrator_still_shows_every_agent", () => {
    const g = groups([member("f1", "specialist", "t", "working", 20), member("f2", "specialist", "t", "review", 30)]);
    expect(g).toHaveLength(1);
    expect(g[0]!.lead.id).toBe("f1");
    expect(ids(g[0]!.fleet)).toEqual(["f2"]);
  });

  it("a_fleet_agent_goes_by_its_personas_nickname", () => {
    const ferris = { ...persona("me"), nicknames: { rust: "Ferris" } };
    expect(fleetName(agent({ persona: "me", role: "rust" }), [ferris])).toBe("Ferris (rust)");
    expect(fleetName(agent({ persona: "me", role: "docs" }), [ferris])).toBe("docs");
    expect(fleetName(agent({ persona: "gone", role: null }), [ferris])).toBe("specialist");
  });
});
