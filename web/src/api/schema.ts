import type { ActionDef } from "./generated/ActionDef";
import type { Agent } from "./generated/Agent";
import type { ColumnDef } from "./generated/ColumnDef";
import type { Condition } from "./generated/Condition";
import type { UiSchema } from "./generated/UiSchema";

/** Mirrors `fleet_api::eval`; the daemon's schema is the only place that says what shows when. */
export function evalCondition(c: Condition, a: Agent): boolean {
  switch (c.type) {
    case "always":
      return true;
    case "status_in":
      return c.statuses.includes(a.status);
    case "paused":
      return a.paused === c.value;
    case "has_changes":
      return a.has_changes === c.value;
    case "all":
      return c.conditions.every((x) => evalCondition(x, a));
  }
}

/**
 * The value a card field shows. Most keys are plain `Agent` fields; `repos` and `branch` are derived from the
 * workspaces, the only place the UI knows how a schema key maps to the agent (the daemon's contract test
 * allows exactly these two derived keys).
 */
export function fieldValue(a: Agent, key: string): unknown {
  if (key === "repos") return a.workspaces.map((w) => w.repo);
  // Every repo of a project sits on the same branch name, so show it once.
  if (key === "branch") return [...new Set(a.workspaces.map((w) => w.branch))];
  return (a as unknown as Record<string, unknown>)[key];
}

export function columnFor(schema: UiSchema, a: Agent): ColumnDef | undefined {
  return schema.columns.find((c) => c.statuses.includes(a.status));
}

export function visibleActions(schema: UiSchema, a: Agent): ActionDef[] {
  return schema.actions.filter((act) => evalCondition(act.when, a));
}

export interface ActionRequest {
  method: string;
  path: string;
  body?: Record<string, unknown>;
}

export function actionRequest(action: ActionDef, agent: Agent, input?: string): ActionRequest {
  const path = action.path.replace("{id}", encodeURIComponent(agent.id));
  if (action.method === "GET" || action.method === "DELETE") return { method: action.method, path };
  const body: Record<string, unknown> = { actor: "local" };
  if (action.input !== null && input !== undefined) body[action.input] = input;
  return { method: action.method, path, body };
}

export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}
