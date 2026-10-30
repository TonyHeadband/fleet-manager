import type { Agent } from "../api/generated/Agent";
import type { Persona } from "../api/generated/Persona";
import type { Release } from "../api/generated/Release";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { Task } from "../api/generated/Task";
import { groups, taskId } from "./groups";

// Lite's Kanban header filters (crates/fleet-gui/src/components/{repo,persona,release}_filter.rs and the filtering in
// app.rs), ported so both boards narrow the same way. None is saved, as in Lite.

/**
 * Lite's `repo_counts`: the configured repos, then any an agent uses that is not configured, each with the number of
 * agents touching it. Counted over every agent, before any filter, so a count says what picking the repo finds.
 */
export function repoRows(repos: RepoInfo[], agents: Agent[]): [string, number][] {
  const counts = new Map<string, number>(repos.map((r) => [r.name, 0]));
  for (const a of agents) {
    for (const repo of new Set(a.workspaces.map((w) => w.repo))) counts.set(repo, (counts.get(repo) ?? 0) + 1);
  }
  return [...counts];
}

export interface PersonaRow {
  id: string;
  label: string;
  count: number;
}

/**
 * What each persona id is shown as (`persona_labels`): its name, the id once the persona is deleted, "no persona"
 * before personas existed, and "Name (id)" where two would read the same.
 */
export function personaLabels(personas: Persona[], ids: string[]): Map<string, string> {
  const label = (id: string) => (id === "" ? "no persona" : (personas.find((p) => p.id === id)?.name ?? id));
  return new Map(
    ids.map((id) => {
      const shared = ids.filter((other) => label(other) === label(id)).length > 1;
      return [id, shared && id !== "" ? `${label(id)} (${id})` : label(id)];
    }),
  );
}

/**
 * Lite's `persona_rows`: the ready personas, then any persona an agent runs under that is not one of them, each with
 * its agent count over every agent. The labels are the cards' tags too.
 */
export function personaRows(personas: Persona[], agents: Agent[]): PersonaRow[] {
  const counts = new Map<string, number>(personas.filter((p) => p.ready).map((p) => [p.id, 0]));
  for (const a of agents) counts.set(a.persona, (counts.get(a.persona) ?? 0) + 1);
  const labels = personaLabels(personas, [...counts.keys()]);
  return [...counts].map(([id, count]) => ({ id, label: labels.get(id) ?? id, count }));
}

/** The persona tag each shown agent's card carries, by persona id; empty under two personas, as a tag only tells
 * cards apart. */
export function personaTags(shown: Agent[], rows: PersonaRow[]): Record<string, string> {
  const ids = [...new Set(shown.map((a) => a.persona))];
  if (ids.length < 2) return {};
  return Object.fromEntries(ids.map((id) => [id, rows.find((r) => r.id === id)?.label ?? id]));
}

export type ReleaseFilter = { type: "all" } | { type: "release"; id: number } | { type: "backlog" };

export function allows(f: ReleaseFilter, release: number | null): boolean {
  switch (f.type) {
    case "all":
      return true;
    case "release":
      return release === f.id;
    case "backlog":
      return release === null;
  }
}

/** The releases a filter offers: those not yet released. */
export function offered(releases: Release[]): Release[] {
  return releases.filter((r) => r.released_at === null);
}

/** `f`, unless it picks a release no longer offered (released, deleted, another product's): then All. */
export function kept(f: ReleaseFilter, releases: Release[]): ReleaseFilter {
  return f.type === "release" && !offered(releases).some((r) => r.id === f.id) ? { type: "all" } : f;
}

/** A task's release by its lead; a task with no record in the list reads as the backlog. */
export function releaseOf(tasks: Task[], lead: Agent): number | null {
  return tasks.find((t) => t.id === taskId(lead))?.release ?? null;
}

export interface Narrowing {
  /** The active product; none shows every product's tasks. */
  product: string | null;
  release: ReleaseFilter;
  /** Empty means every repo. */
  repos: string[];
  /** Persona ids; empty means every persona. */
  personas: string[];
  tasks: Task[];
}

/**
 * The agents the board shows, as app.rs narrows them: a task passes by its lead (product, release, repo, persona, in
 * that order), and its fleet comes along, so a fleet agent never shows without it. They keep the snapshot's order:
 * the board groups them again, and a task's lead is chosen by that order.
 */
export function shownAgents(agents: Agent[], n: Narrowing): Agent[] {
  const kept = new Set(
    groups(agents)
      .filter(({ lead }) => n.product === null || lead.product === n.product)
      .filter(({ lead }) => allows(n.release, releaseOf(n.tasks, lead)))
      .filter(({ lead }) => n.repos.length === 0 || lead.workspaces.some((w) => n.repos.includes(w.repo)))
      .filter(({ lead }) => n.personas.length === 0 || n.personas.includes(lead.persona))
      .map(({ lead }) => taskId(lead)),
  );
  return agents.filter((a) => kept.has(taskId(a)));
}
