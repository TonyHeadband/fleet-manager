import { describe, expect, it } from "vitest";
import type { Release } from "../api/generated/Release";
import type { Task } from "../api/generated/Task";
import { agent, agentIn, persona, repoInfo } from "../test/fixtures";
import { allows, kept, offered, personaRows, personaTags, releaseOf, repoRows, shownAgents } from "./filters";

// Ported from crates/fleet-gui/tests/panels.rs and app.rs; the names are Lite's.
const release = (id: number, released_at: number | null = null): Release => ({ id, product: "p1", version: `${id}.0`, start: "", target: "", released_at });
const taskIn = (id: string, rel: number | null) => ({ id, release: rel }) as unknown as Task;
const none = { product: null, release: { type: "all" } as const, repos: [], personas: [], tasks: [] };

describe("filters", () => {
  it("the_repo_filter_lists_configured_repos_then_any_an_agent_uses", () => {
    const repos = [repoInfo("core"), repoInfo("portal", { product: "p2" }), repoInfo("core", { product: "p2" })];
    const agents = [agentIn("core", { id: "a" }), agent({ id: "b", workspaces: [agentIn("core").workspaces[0]!, agentIn("lab").workspaces[0]!] })];
    expect(repoRows(repos, agents)).toEqual([
      ["core", 2],
      ["portal", 0],
      ["lab", 1],
    ]);
  });

  it("personas_with_one_name_stay_apart_in_the_filter_and_the_tags", () => {
    const people = [{ ...persona("p"), name: "Ann" }, { ...persona("w"), name: "Ann" }, { ...persona("x", false), name: "Xi" }];
    const agents = [agent({ id: "a", persona: "p" }), agent({ id: "b", persona: "w" }), agent({ id: "c", persona: "w" }), agent({ id: "d", persona: "" })];
    expect(personaRows(people, agents)).toEqual([
      { id: "p", label: "Ann (p)", count: 1 },
      { id: "w", label: "Ann (w)", count: 2 },
      { id: "", label: "no persona", count: 1 },
    ]);
    expect(personaTags(agents, personaRows(people, agents))).toEqual({ p: "Ann (p)", w: "Ann (w)", "": "no persona" });
  });

  it("card_tag_only_when_several_personas_visible", () => {
    const people = [persona("p"), persona("w")];
    const shown = [agent({ id: "a", persona: "p" })];
    expect(personaTags(shown, personaRows(people, [...shown, agent({ id: "b", persona: "w" })]))).toEqual({});
  });

  it("release_filter_backlog_hides_assigned_tasks", () => {
    const tasks = [taskIn("t1", 1), taskIn("t2", null)];
    expect(releaseOf(tasks, agent({ id: "t1" }))).toBe(1);
    expect(releaseOf(tasks, agent({ id: "t2" }))).toBeNull();
    expect(releaseOf(tasks, agent({ id: "unknown" })), "no record reads as the backlog").toBeNull();
    expect(allows({ type: "backlog" }, 1)).toBe(false);
    expect(allows({ type: "backlog" }, null)).toBe(true);
    expect(allows({ type: "release", id: 1 }, 1)).toBe(true);
    expect(allows({ type: "release", id: 1 }, null)).toBe(false);
    expect(allows({ type: "all" }, 7)).toBe(true);
  });

  it("a_release_no_longer_offered_falls_back_to_all", () => {
    const releases = [release(1), release(2, 1700)];
    expect(offered(releases).map((r) => r.id)).toEqual([1]);
    expect(kept({ type: "release", id: 1 }, releases)).toEqual({ type: "release", id: 1 });
    expect(kept({ type: "release", id: 2 }, releases), "released").toEqual({ type: "all" });
    expect(kept({ type: "release", id: 9 }, releases), "another product's").toEqual({ type: "all" });
    expect(kept({ type: "backlog" }, [])).toEqual({ type: "backlog" });
  });

  it("the_shown_agents_keep_the_snapshots_order", () => {
    // The board groups them again: a restarted orchestrator must still lead after the filters.
    const agents = [
      agent({ id: "o", kind: "orchestrator", status: "stopped", stop_cause: "stopped" }),
      agent({ id: "o2", kind: "orchestrator", task: "o", status: "stopped", stop_cause: "crashed" }),
    ];
    expect(shownAgents(agents, none).map((a) => a.id)).toEqual(["o", "o2"]);
  });

  it("a_fleet_passes_the_filters_by_its_lead", () => {
    const lead = agentIn("core", { id: "o", kind: "orchestrator", persona: "p" });
    const fleet = agentIn("portal", { id: "f", kind: "specialist", task: "o", persona: "w" });
    const solo = agentIn("portal", { id: "s", persona: "w" });
    const g = [lead, fleet, solo];
    expect(shownAgents(g, { ...none, repos: ["core"] }).map((a) => a.id)).toEqual(["o", "f"]);
    expect(shownAgents(g, { ...none, personas: ["w"] }).map((a) => a.id)).toEqual(["s"]);
    expect(shownAgents(g, { ...none, product: "other" })).toEqual([]);
    expect(shownAgents(g, none).map((a) => a.id)).toEqual(["o", "f", "s"]);
  });
});
