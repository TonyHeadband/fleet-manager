import type { CheckoutInfo } from "../api/generated/CheckoutInfo";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import type { Release } from "../api/generated/Release";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { RepoPick } from "../api/generated/RepoPick";
import type { ToolError } from "../api/generated/ToolError";
import type { ToolId } from "../api/generated/ToolId";
import type { WorkflowState } from "../api/generated/WorkflowState";

/** Where a picked repo starts: `null` from its URL, a path from that local checkout. No entry: its URL, else its main checkout. */
export type Start = string | null;

/** What the New task dialog holds across openings, as Lite's `SpawnState`. */
export interface Draft {
  persona: string;
  prompt: string;
  /** The card's title; empty leaves the naming to the agent. */
  title: string;
  /** A release of the product; null is the backlog. */
  release: number | null;
  /** The orchestrator type; null is Solo. */
  role: string | null;
  chosen: string[];
  starts: Record<string, Start>;
  /** Why repos picked under Solo were dropped on the switch to an orchestrator type. */
  dropped: string | null;
  /** The goal the last spawn sent: a success clears the dialog only while it still holds this text. */
  sent: string | null;
  busy: boolean;
  error: string | null;
  /** The tool the daemon refused the last spawn for, shown with its fix in place of `error`. */
  refusal: ToolError | null;
}

export type Listing = { state: "loading" } | { state: "ready"; checkouts: CheckoutInfo[] } | { state: "failed"; error: string };
/** Each local folder's checkouts, by path. */
export type Listings = Record<string, Listing>;

export function emptyDraft(): Draft {
  return { persona: "", prompt: "", title: "", release: null, role: null, chosen: [], starts: {}, dropped: null, sent: null, busy: false, error: null, refusal: null };
}

/** `null` is Solo. An orchestrated task has one repo: the first pick is kept and the others are dropped, which the dialog says. */
export function setType(d: Draft, role: string | null): Draft {
  if (role === null) return { ...d, role: null, dropped: null };
  if (d.chosen.length <= 1) return { ...d, role, dropped: null };
  const rest = d.chosen.slice(1);
  const verb = rest.length === 1 ? "was" : "were";
  return { ...d, role, chosen: d.chosen.slice(0, 1), dropped: `An orchestrated task takes one repo: ${rest.join(", ")} ${verb} dropped.` };
}

/** After a pick under an orchestrator type: the newest pick replaces the old one. */
export function keepOneRepo(d: Draft, before: string[]): Draft {
  const newest = [...d.chosen].reverse().find((c) => !before.includes(c));
  const chosen = newest === undefined ? d.chosen : [newest];
  const changed = chosen.length !== before.length || chosen.some((c, i) => c !== before[i]);
  return { ...d, chosen, dropped: changed ? null : d.dropped };
}

/** Moves the dialog to another persona: the picks go, and the repos are fetched again. */
export function setPersona(d: Draft, id: string): Draft {
  return id === d.persona ? d : { ...d, persona: id, chosen: [], starts: {} };
}

/** Forgets the per-task picks once a task started. */
export const sentOk = (d: Draft): Draft => ({ ...d, dropped: null, release: null, starts: {} });

/** On each opening: the backlog, and each repo from its default start. */
export const resetChoices = (d: Draft): Draft => ({ ...d, release: null, starts: {} });

/** Whether `name` starts from a local checkout: picked so, or its only way to start. */
export function wantsLocal(repos: RepoInfo[], d: Draft, name: string): boolean {
  const info = repos.find((r) => r.name === name);
  if (!info?.path) return false;
  return info.url === null || (d.starts[name] ?? null) !== null;
}

/** The path `name` starts from, `null` from its URL. A pick of the repo's own path means its main checkout. */
export function startOf(repos: RepoInfo[], d: Draft, listings: Listings, name: string): string | null {
  if (!wantsLocal(repos, d, name)) return null;
  const path = repos.find((r) => r.name === name)!.path!;
  const picked = d.starts[name] ?? null;
  if (picked !== null && picked !== path) return picked;
  const listing = listings[path];
  return (listing?.state === "ready" ? listing.checkouts.find((c) => c.main)?.path : undefined) ?? path;
}

/** Why a start cannot be sent yet: a local start whose checkouts are still loading, failed to list, or are none. */
export function startBlocker(repos: RepoInfo[], d: Draft, listings: Listings): string | null {
  for (const name of d.chosen) {
    if (!wantsLocal(repos, d, name)) continue;
    const listing = listings[repos.find((r) => r.name === name)!.path!];
    if (!listing || listing.state === "loading") return `Loading the checkouts of ${name}…`;
    if (listing.state === "failed") return `Cannot list the checkouts of ${name}: ${listing.error}`;
    if (listing.checkouts.length === 0) return `${name} has no checkout to start from`;
  }
  return null;
}

export function picks(repos: RepoInfo[], d: Draft, listings: Listings): RepoPick[] {
  return d.chosen.map((repo) => ({ repo, local: startOf(repos, d, listings, repo) }));
}

/** Why Start cannot run under `persona`, if it cannot. */
export function personaBlocker(personas: Persona[], persona: string): string | null {
  if (personas.some((p) => p.ready && p.id === persona)) return null;
  return personas.some((p) => p.ready) ? "Pick a persona" : "Set up a persona first";
}

/** Why an orchestrated task cannot start yet. The workflow line never blocks: creation fetches and refuses. */
export const fleetBlocker = (d: Draft): string | null => (d.role !== null && d.chosen.length === 0 ? "Pick a repo" : null);

/** What a repo's workflow state says, and whether it is a problem to show (not a block: creation decides). */
export function workflowLine(repo: string, w: WorkflowState): { text: string; blocks: boolean } {
  switch (w.state) {
    case "found":
      return { text: `Checks on ${w.base}: ${w.steps.join(", ")}`, blocks: false };
    case "missing":
      return {
        text: `${repo} has no .fleet/workflow.yml on ${w.base}. It lists the checks the gate runs before a pull request, so orchestrated tasks need it.`,
        blocks: true,
      };
    case "invalid":
      return { text: `${repo}: .fleet/workflow.yml on ${w.base} is invalid: ${w.error}`, blocks: true };
    case "unreadable":
      return { text: `${repo}: cannot read .fleet/workflow.yml on ${w.base}: ${w.error}`, blocks: true };
  }
}

/** The last two parts of a long path: enough to tell checkouts apart. */
export function shortPath(path: string): string {
  const parts = path.split(/[/\\]/).filter((p, i) => p !== "" || i === 0);
  // As Rust's `Path::components`: the root is one ("/a/b" is three), and a Windows drive adds a prefix before it.
  const components = parts.length + (/^[A-Za-z]:[/\\]/.test(path) ? 1 : 0);
  if (components <= 3) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

export const checkoutLabel = (c: CheckoutInfo) => `${c.branch ?? "detached"}  ${shortPath(c.path)}`;

export const openReleases = (product: Product | undefined): Release[] => product?.releases.filter((r) => r.released_at === null) ?? [];

/** A release that is gone (the product changed, or it was released) is not sent under the Backlog label. */
export const keptRelease = (product: Product | undefined, id: number | null): number | null =>
  id !== null && openReleases(product).some((r) => r.id === id) ? id : null;

const TOOL_NAMES: Record<ToolId, string> = { git: "Git", "git-bash": "Git Bash", claude: "Claude Code", gh: "GitHub CLI (gh)" };

/** Lite's `ToolError` display. */
export function toolErrorText(e: ToolError): string {
  const name = TOOL_NAMES[e.tool];
  switch (e.kind) {
    case "not-installed":
      return `${name} is not installed`;
    case "too-old":
      return `${name} ${e.found} is too old: ${e.needed} or newer is needed`;
    case "not-logged-in":
      return e.persona === null ? `${name} is not logged in` : `${name} is not logged in for ${e.persona}`;
    case "broken":
      return `${name} is installed but does not work: ${e.detail}`;
  }
}
