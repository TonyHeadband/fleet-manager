import { describe, expect, it } from "vitest";
import { actionRequest, columnFor, evalCondition, fieldValue, formatDuration, shortTitle, visibleActions } from "./schema";
import type { Condition } from "./generated/Condition";
import type { UiSchema } from "./generated/UiSchema";
import { agent, schema, workspace } from "../test/fixtures";

const ids = (s: UiSchema, a: ReturnType<typeof agent>) => visibleActions(s, a).map((x) => x.id);

describe("columnFor", () => {
  it("columnFor_uses_schema_status_mapping", () => {
    expect(columnFor(schema, agent({ status: "stopped" }))?.id).toBe("stopped");
    expect(columnFor(schema, agent({ status: "needs_you" }))?.id).toBe("needs_you");
    const custom: UiSchema = { ...schema, columns: [{ id: "x", title: "X", statuses: ["working", "review"], empty_hint: "", tone: "neutral", dim: false }] };
    expect(columnFor(custom, agent({ status: "review" }))?.id).toBe("x");
    expect(columnFor(custom, agent({ status: "queued" }))).toBeUndefined();
  });
});

describe("evalCondition", () => {
  const a = agent({ status: "working", paused: false, has_changes: true });
  it("mirrors the daemon's eval", () => {
    expect(evalCondition({ type: "always" }, a)).toBe(true);
    expect(evalCondition({ type: "status_in", statuses: ["working"] }, a)).toBe(true);
    expect(evalCondition({ type: "status_in", statuses: ["review"] }, a)).toBe(false);
    expect(evalCondition({ type: "paused", value: true }, a)).toBe(false);
    expect(evalCondition({ type: "has_changes", value: true }, a)).toBe(true);
    expect(evalCondition({ type: "all", conditions: [{ type: "always" }, { type: "paused", value: false }] }, a)).toBe(true);
    expect(evalCondition({ type: "all", conditions: [{ type: "always" }, { type: "paused", value: true }] }, a)).toBe(false);
  });
});

describe("evalCondition blocked reasons", () => {
  const answerable: Condition = { type: "all", conditions: [{ type: "status_in", statuses: ["needs_you"] }, { type: "not", condition: { type: "blocked_reason_in", reasons: ["repo_access"] } }] };
  it("shows Answer for a permission request and hides it for repo_access", () => {
    expect(evalCondition(answerable, agent({ status: "needs_you", blocked_reason: "permission" }))).toBe(true);
    expect(evalCondition(answerable, agent({ status: "needs_you", blocked_reason: "repo_access" }))).toBe(false);
    expect(ids(schema, agent({ status: "needs_you", blocked_reason: "permission" }))).toContain("answer");
    expect(ids(schema, agent({ status: "needs_you", blocked_reason: "repo_access" }))).not.toContain("answer");
  });
  it("blocked_reason_in is false when there is no reason", () => {
    expect(evalCondition({ type: "blocked_reason_in", reasons: ["permission"] }, agent({ status: "working", blocked_reason: null }))).toBe(false);
  });
});

describe("visibleActions", () => {
  it("visibleActions_follows_conditions", () => {
    expect(ids(schema, agent({ status: "working" }))).toEqual(["pause", "stop"]);
    // No Pause on a Needs you card: Escape would deny the pending request.
    expect(ids(schema, agent({ status: "needs_you" }))).toEqual(["answer", "stop"]);
    expect(ids(schema, agent({ status: "review", paused: true }))).toEqual(["resume", "stop"]);
    // A stopped agent has nothing left to stop.
    expect(ids(schema, agent({ status: "stopped" }))).toEqual([]);
    expect(ids(schema, agent({ status: "stopped", paused: true }))).toEqual([]);
  });
});

describe("fieldValue", () => {
  it("fieldValue_derives_repos_and_branch_from_workspaces", () => {
    const a = agent({ workspaces: [workspace("core"), workspace("portal")] });
    expect(fieldValue(a, "repos")).toEqual(["core", "portal"]);
    expect(fieldValue(a, "branch")).toEqual(["fleet/x"]); // one branch name across the task's repos, shown once
    expect(fieldValue(a, "goal")).toBe(a.goal);
    expect(fieldValue(a, "no_such_field")).toBeUndefined();
  });
  it("fieldValue_title_is_the_named_one_or_a_cut_of_the_goal", () => {
    expect(fieldValue(agent({ goal: "Fix the login. Then the logs" }), "title")).toBe("Fix the login");
    expect(fieldValue(agent({ goal: "x", title: "Flaky tests" }), "title")).toBe("Flaky tests");
  });
  it("shortTitle_matches_the_rust_cut", () => {
    expect(shortTitle("why does it crash? it did yesterday")).toBe("why does it crash?");
    expect(shortTitle("bump v1.2 in Cargo.toml")).toBe("bump v1.2 in Cargo.toml");
    expect(shortTitle("when a task starts the card title is the whole prompt and it does not fit on the card")).toBe(
      "when a task starts the card title is the whole prompt and…",
    );
    expect(shortTitle("x".repeat(80))).toBe(`${"x".repeat(60)}…`);
  });
});

describe("actionRequest", () => {
  const find = (id: string) => schema.actions.find((a) => a.id === id)!;
  it("builds method, path and body from the action definition", () => {
    expect(actionRequest(find("stop"), agent({ id: "abc" }))).toEqual({ method: "POST", path: "/api/agents/abc/stop", body: { actor: "local" } });
    expect(actionRequest(find("pause"), agent({ id: "abc" }))).toEqual({ method: "POST", path: "/api/agents/abc/pause", body: { actor: "local" } });
    expect(actionRequest(find("answer"), agent({ id: "abc" }), '$(x) "q"')).toEqual({
      method: "POST",
      path: "/api/agents/abc/answer",
      body: { actor: "local", text: '$(x) "q"' },
    });
  });
  it("escapes the agent id in the path", () => {
    expect(actionRequest(find("stop"), agent({ id: "a/b?c" })).path).toBe("/api/agents/a%2Fb%3Fc/stop");
  });
});

describe("formatDuration", () => {
  it("shows the two largest units", () => {
    expect([0, 5, 65, 3700, 90000, -4].map(formatDuration)).toEqual(["0s", "5s", "1m", "1h 1m", "1d 1h", "0s"]);
  });
});
