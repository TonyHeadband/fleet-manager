import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { App } from "../App";
import type { Native } from "../native/native";
import type { Agent } from "../api/generated/Agent";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import type { LoginState } from "../api/generated/LoginState";
import type { Persona } from "../api/generated/Persona";
import type { PersonaUpdate } from "../api/generated/PersonaUpdate";
import type { Product } from "../api/generated/Product";
import type { Release } from "../api/generated/Release";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { Task } from "../api/generated/Task";
import { FakeEventSource, factory, fakeClient, fullTask } from "../test/fixtures";

// Management's test harness, after egui's tests/management.rs helpers.

/** egui's TODAY: the day the release bars are drawn on. */
export const TODAY = "2026-10-11";

/** Local noon of `date`. */
export function noon(date: string) {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return new Date(y, m - 1, d, 12).getTime() / 1000;
}

export function release(id: number, product: string, version: string, released: boolean): Release {
  return { id, product, version, start: "2026-10-01", target: "2026-11-01", released_at: released ? noon("2026-10-05") : null };
}

export function managedProduct(id: string, name: string, releases: Release[] = [], over: Partial<Product> = {}): Product {
  return {
    id,
    name,
    default_persona: null,
    internal_url: null,
    repos: [],
    releases,
    tasks: 0,
    overrides: [],
    hidden_specialists: [],
    ...over,
  } as Product;
}

/** egui's `repo`: a repo of product p, its workflow file missing on main, with no env. */
export const managedRepo = (name: string, path: string | null, url: string | null, over: Partial<RepoInfo> = {}): RepoInfo => ({
  product: "p",
  name,
  path,
  url,
  workflow: { state: "missing", base: "main" },
  env: [],
  ...over,
});

/** egui's `def`: a built-in definition, unusable when `error` says why. */
export const managedDef = (kind: DefKind, name: string, error: string | null = null): DefInfo => ({
  kind,
  name,
  summary: "",
  builtin: true,
  overridden: false,
  error,
});

/** A task of `product`, running unless said otherwise, in `release`. */
export const managedTask = (id: string, product: string, state: Task["state"] = "running", release: number | null = null) =>
  fullTask(id, id, state, state === "running" ? null : "stopped", { product, release });

export function mergedTask(id: string, product: string, release: number | null): Task {
  const t = fullTask(id, id, "archived", "stopped", {
    product,
    release,
    prs: [{ url: "u", repo: "core", state: "merged", merged_at: null }],
  });
  t.repos[0]!.changed = true;
  return t;
}

/** egui's `two_products`: Alpha with an unreleased 1.0 and a released 0.9, and Beta with none. */
export const twoProducts = () => [
  managedProduct("a", "Alpha", [release(1, "a", "1.0", false), release(2, "a", "0.9", true)]),
  managedProduct("b", "Beta"),
];

/** egui's `counting_tasks`, with each task's agent on the board. */
export function countingTasks(): [Task[], Partial<Agent>[]] {
  return [
    [
      managedTask("a1", "a", "running", 1),
      managedTask("a2", "a"),
      managedTask("a3", "a"),
      mergedTask("a4", "a", 1),
      // Stopped, so no live agent: counted nowhere.
      managedTask("a5", "a", "stopped"),
      managedTask("b1", "b"),
      managedTask("b2", "b"),
      managedTask("b3", "b"),
    ],
    [
      { id: "a1", product: "a", status: "working" },
      { id: "a2", product: "a", status: "working" },
      { id: "a3", product: "a", status: "needs_you" },
      { id: "b1", product: "b", status: "working" },
      { id: "b2", product: "b", status: "review" },
      { id: "b3", product: "b", status: "review" },
    ],
  ];
}

/** egui's `products_harness`: the app on Management, then on `tab`, with these products, tasks, agents and personas. */
export async function management(
  tab: string,
  {
    products = twoProducts(),
    tasks = [] as Task[],
    agents = [] as Partial<Agent>[],
    personas = [managedPersona("a", "Work", true)],
    native = null as Native | null,
    local = false,
    over = {} as Parameters<typeof fakeClient>[0],
  } = {},
) {
  FakeEventSource.reset();
  window.location.hash = "#kanban";
  const client = fakeClient({
    personas: vi.fn().mockResolvedValue({
      personas,
      claude_command: ["claude"],
      gh_command: "gh",
      tasks_root: "/tasks",
      claude_path: null,
      gh_path: null,
    }),
    // A check answers with the persona as listed, as a daemon with nothing changed would; one made since is a draft.
    checkPersona: vi.fn(async (id: string) => personas.find((p) => p.id === id) ?? managedPersona(id, id, false)),
    // An update answers with the listed persona changed, as the daemon does.
    updatePersona: vi.fn(async (id: string, u: PersonaUpdate) => {
      const p = personas.find((l) => l.id === id) ?? managedPersona(id, id, true);
      const set = Object.fromEntries(Object.entries(u).filter(([, v]) => v !== null));
      return { ...p, ...set } as Persona;
    }),
    // The same as fakeNative's, so the Tauri app's version offer stays away.
    version: vi.fn().mockResolvedValue({ version: "0.2.0" }),
    products: vi.fn().mockResolvedValue({ products }),
    tasks: vi.fn().mockResolvedValue({ tasks, orphans: [], clones_bytes: null }),
    ...over,
  });
  const wizard = { steps: { folders: false, requirements: false }, stillMissing: [], local, weStarted: false };
  render(<App client={client} eventSource={factory} nowSeconds={() => noon(TODAY)} native={native} wizard={wizard} />);
  const full = agents.map((a) => ({ ...(defaultAgent as object), ...a }));
  act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: full }, "1"));
  await click("Management");
  if (tab !== "Overview") await click(tab);
  return client;
}

/** egui's `login`: ok as "ann", or not with "token expired". */
export const login = (ok: boolean): LoginState => ({ ok, account: ok ? "ann" : null, detail: ok ? null : "token expired", tool: null });

/** egui's `persona`: Ann Example on github.com with gh login; a ready one has both logins ok. */
export function managedPersona(id: string, name: string, ready: boolean, over: Partial<Persona> = {}): Persona {
  return {
    id,
    name,
    host: "github.com",
    commit_name: "Ann Example",
    commit_email: "ann@example.invalid",
    github_auth: "gh",
    claude_dir: `/personas/${id}/claude`,
    gh_dir: `/personas/${id}/gh`,
    ready,
    claude: ready ? login(true) : null,
    github: ready ? login(true) : null,
    tasks: 0,
    nicknames: {},
    ...over,
  };
}

const defaultAgent: Partial<Agent> = {
  goal: "g",
  workdir: "/w",
  workspaces: [],
  session: "s",
  activity: "",
  status_since: 0,
  has_changes: false,
  paused: false,
  owner: "o",
  blocked_reason: null,
  persona: "a",
  kind: "solo",
};

/** egui's `click`: a drawer link, a tab, or a button, by its name. */
export async function click(name: string) {
  const find = () => {
    const target = [
      ...screen.queryAllByRole("link", { name }),
      ...screen.queryAllByRole("tab", { name }),
      ...screen.queryAllByRole("button", { name }),
    ][0];
    if (!target) throw new Error(`nothing named ${name}`);
    return target;
  };
  await userEvent.click(await waitFor(find));
}

/** A promise answered by hand, as egui's tests hand results to the app. */
export function pending<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
