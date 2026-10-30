import { vi } from "vitest";
import type { FleetClient } from "../api/client";
import type { CloseEvent, Native } from "../native/native";
import type { DefKind } from "../api/generated/DefKind";
import type { FormDef } from "../api/generated/FormDef";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import type { ProductOverride } from "../api/generated/ProductOverride";
import type { StopCause } from "../api/generated/StopCause";
import type { Task } from "../api/generated/Task";
import type { TaskState } from "../api/generated/TaskState";
import type { TaskList } from "../api/generated/TaskList";
import type { Agent } from "../api/generated/Agent";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { UiSchema } from "../api/generated/UiSchema";
import defaultSchema from "../api/generated/default-ui-schema.json";

// Through `unknown`: the JSON's inferred type is narrower than UiSchema in places (fixed values of mixed kinds).
export const schema = defaultSchema as unknown as UiSchema;

export function workspace(repo: string, has_changes = false) {
  return { repo, branch: "fleet/x", worktree: `/w/x/${repo}`, base: "abc123", has_changes, owned: false };
}

/** An agent whose single workspace is `repo`. */
export function agentIn(repo: string, over: Partial<Agent> = {}): Agent {
  return agent({ workspaces: [workspace(repo)], ...over });
}

export function agent(over: Partial<Agent> = {}): Agent {
  return {
    id: "a1",
    goal: "fix the tests",
    workdir: "/w/fix-the-tests",
    workspaces: [
      { repo: "core", branch: "fleet/fix-the-tests", worktree: "/w/fix-the-tests/core", base: "abc123", has_changes: false, owned: false },
    ],
    session: "fleet-a1",
    status: "working",
    activity: "running Bash",
    status_since: 1000,
    has_changes: false,
    paused: false,
    owner: "antonio",
    blocked_reason: null,
    ...over,
  } as Agent;
}

/** Stand-in for the browser's EventSource that tests drive by hand. */
export class FakeEventSource {
  static instances: FakeEventSource[] = [];
  closed = false;
  private handlers = new Map<string, Array<(e: MessageEvent) => void>>();
  constructor(public url: string) {
    FakeEventSource.instances.push(this);
  }
  addEventListener(type: string, fn: (e: MessageEvent) => void) {
    this.handlers.set(type, [...(this.handlers.get(type) ?? []), fn]);
  }
  emit(type: string, data: unknown, id?: string) {
    const ev = new MessageEvent(type, { data: JSON.stringify(data), lastEventId: id ?? "" });
    for (const fn of this.handlers.get(type) ?? []) fn(ev);
  }
  close() {
    this.closed = true;
  }
  static reset() {
    FakeEventSource.instances = [];
  }
  static for(pathPart: string): FakeEventSource {
    const es = FakeEventSource.instances.find((i) => i.url.includes(pathPart));
    if (!es) throw new Error(`no EventSource for ${pathPart}: ${FakeEventSource.instances.map((i) => i.url)}`);
    return es;
  }
}

export function form(id: string): FormDef {
  return structuredClone(schema.forms.find((f) => f.id === id)!);
}

export function repoInfo(name: string, over: Partial<RepoInfo> = {}): RepoInfo {
  return {
    product: "p1",
    name,
    path: null,
    url: `https://example.test/${name}.git`,
    workflow: { state: "missing", base: "main" },
    env: [],
    ...over,
  };
}

export const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;

export function fakeClient(over: Partial<Record<keyof FleetClient, unknown>> = {}): FleetClient {
  return {
    baseUrl: "",
    uiSchema: vi.fn().mockResolvedValue(schema),
    repos: vi.fn().mockResolvedValue([repoInfo("core"), repoInfo("portal")]),
    personas: vi.fn().mockResolvedValue({ personas: [] }),
    createPersona: vi.fn(async (p: { name: string }) => persona(p.name, false)),
    checkPersona: vi.fn(async (id: string) => persona(id, false)),
    deletePersona: vi.fn().mockResolvedValue(undefined),
    updatePersona: vi.fn(async (id: string) => persona(id)),
    products: vi.fn().mockResolvedValue({ products: [] }),
    createProduct: vi.fn(async (p: { name: string }) => product(p.name.toLowerCase(), p.name)),
    updateProduct: vi.fn(async (id: string) => product(id)),
    upsertRepo: vi.fn().mockResolvedValue({}),
    deleteRepo: vi.fn().mockResolvedValue(undefined),
    deleteProduct: vi.fn().mockResolvedValue(undefined),
    // Each answers with what was sent stored, as the daemon does.
    putHiddenSpecialists: vi.fn(async (id: string, hidden: string[]) => product(id, id, { hidden_specialists: hidden, overrides: [] })),
    putOverrides: vi.fn(async (id: string, overrides: ProductOverride[]) => product(id, id, { overrides, hidden_specialists: [] })),
    effectiveSettings: vi.fn().mockResolvedValue({ values: [] }),
    internalBranches: vi.fn().mockResolvedValue({ branches: [], remote: { url: null, unpushed: 0, error: null, parked: [] } }),
    internalFiles: vi.fn().mockResolvedValue([]),
    internalFile: vi.fn().mockResolvedValue(""),
    internalHistory: vi.fn().mockResolvedValue([]),
    tasks: vi.fn().mockResolvedValue({ tasks: [], orphans: [], clones_bytes: null }),
    refreshTasks: vi.fn().mockResolvedValue(undefined),
    archivePlan: vi.fn().mockResolvedValue({ items: [], total: 0 }),
    archive: vi.fn().mockResolvedValue({ removed: [], freed: 0, kept: null }),
    orphanPlan: vi.fn().mockResolvedValue({ items: [], total: 0 }),
    archiveOrphan: vi.fn().mockResolvedValue({ removed: [], freed: 0, kept: null }),
    markDone: vi.fn(async (id: string) => task(id, "stopped", null, 0)),
    setTaskRelease: vi.fn(async (id: string) => task(id, "stopped", null, 0)),
    updateRelease: vi.fn().mockResolvedValue({}),
    deleteRelease: vi.fn().mockResolvedValue(undefined),
    spawn: vi.fn().mockResolvedValue({ agent: agent(), warnings: [] }),
    run: vi.fn().mockResolvedValue(undefined),
    diff: vi.fn().mockResolvedValue({ workspaces: [] }),
    definitions: vi.fn().mockResolvedValue([]),
    definition: vi.fn(async (kind: DefKind, name: string) => ({ kind, name, text: "", builtin: true })),
    saveDefinition: vi.fn(async (kind: DefKind, name: string) => ({ kind, name, summary: "", builtin: false, overridden: false })),
    resetDefinition: vi.fn().mockResolvedValue(undefined),
    checkouts: vi.fn().mockResolvedValue([]),
    fleetView: vi.fn().mockResolvedValue({ task: "", breakdowns: [], gate_runs: [] }),
    agentConfig: vi.fn().mockResolvedValue({ instructions: "", settings: { allow: [], deny: [], env: {} } }),
    saveAgentConfig: vi.fn(async (u: { instructions?: string }) => ({ instructions: u.instructions ?? "", settings: { allow: [], deny: [], env: {} }, settings_error: null })),
    ...over,
  } as unknown as FleetClient;
}

export function persona(id: string, ready = true): Persona {
  return { id, name: id, ready } as Persona;
}

export function product(id: string, name = id, over: Partial<Product> = {}): Product {
  return { id, name, default_persona: null, repos: [], releases: [], ...over } as Product;
}

/** A task holding `bytes` on disk. */
export function task(id: string, state: TaskState, stop_cause: StopCause | null, bytes: number): Task {
  return { id, state, stop_cause, footprint: { measured_at: 0, items: [], total: bytes } } as unknown as Task;
}

/** The Tauri bridge with every call answering as on a working machine. */
export function fakeNative(over: Partial<Native> = {}): Native {
  return {
    version: vi.fn().mockResolvedValue("0.2.0"),
    hasConfig: vi.fn().mockResolvedValue(true),
    startDaemon: vi.fn().mockResolvedValue({ weStarted: true, url: "http://127.0.0.1:45123", local: true }),
    stateDir: vi.fn().mockResolvedValue("/home/me/.local/share/fleet-manager"),
    firstRunDefaults: vi.fn().mockResolvedValue({ root: "~/fleet-tasks", personas: "~/.local/share/fleet-manager/personas" }),
    writeInitialConfig: vi.fn().mockResolvedValue(undefined),
    pickFolder: vi.fn().mockResolvedValue(null),
    installTool: vi.fn().mockResolvedValue(undefined),
    adoptPath: vi.fn().mockResolvedValue(undefined),
    copyText: vi.fn().mockResolvedValue(undefined),
    runLogin: vi.fn().mockResolvedValue({ script: null }),
    savePat: vi.fn().mockResolvedValue(undefined),
    addMenuShortcut: vi.fn().mockResolvedValue(undefined),
    openUrl: vi.fn().mockResolvedValue(undefined),
    onClose: vi.fn(() => () => {}),
    answerClose: vi.fn().mockResolvedValue(undefined),
    setCloseArmed: vi.fn().mockResolvedValue(undefined),
    openTerminal: vi.fn().mockResolvedValue(undefined),
    restartDaemon: vi.fn().mockResolvedValue({ weStarted: true, url: "http://127.0.0.1:45123", local: true }),
    checkUpdate: vi.fn().mockResolvedValue(null),
    skipVersion: vi.fn().mockResolvedValue(undefined),
    readChoices: vi.fn().mockResolvedValue({ persona: null, product: null, display: "compact" }),
    writeChoice: vi.fn().mockResolvedValue(undefined),
    ...over,
  } as Native;
}

/** A bridge whose window close requests the test sends by hand, as the Rust side emits them. */
export function closingNative(over: Partial<Native> = {}) {
  let handler: ((e: CloseEvent) => void) | null = null;
  const native = fakeNative({
    onClose: vi.fn((h: (e: CloseEvent) => void) => {
      handler = h;
      return () => (handler = null);
    }),
    ...over,
  });
  return { native, emit: (e: CloseEvent) => handler?.(e) };
}

export const NOW = 1_800_000_000;

/** egui's tests/tasks.rs `task`: a solo task in product "p" under persona "p", one repo, 8 GB on disk. */
export function fullTask(id: string, name: string, state: TaskState, stop_cause: StopCause | null, over: Partial<Task> = {}): Task {
  return {
    id,
    product: "p",
    release: null,
    name,
    goal: `${name} goal\nmore detail`,
    title: null,
    workdir: `/p/${name}`,
    repos: [
      {
        repo: "core",
        branch: "fleet/core-x",
        worktree: "/p/x/core",
        base: "abcdef1234",
        from_branch: "main",
        has_changes: false,
        owned: false,
        changed: false,
      },
    ],
    state,
    stop_cause,
    created_at: NOW - 7200,
    stopped_at: state !== "running" ? NOW - 600 : null,
    archived_at: state === "archived" ? NOW - 60 : null,
    agents_used: 1,
    times: { queued: 5, working: 3600, needs_you: 120, review: 0 },
    log: [{ at: NOW - 7200, text: "started with core" }],
    footprint: { measured_at: NOW - 300, items: [{ label: "core", path: `/p/${name}/core`, bytes: 8e9, build_bytes: 7.6e9 }], total: 8e9 },
    last_screen: null,
    archive: null,
    persona: "p",
    kind: "solo",
    breakdowns: [],
    prs: [],
    done_at: null,
    done_by_hand: false,
    ...over,
  } as Task;
}

/** egui's tests/tasks.rs `list`: one running, two stopped (one crashed), one archived, and an orphan folder. */
export function tasksList(): TaskList {
  return {
    tasks: [
      fullTask("r1", "alpha", "running", null),
      fullTask("s1", "bravo", "stopped", "stopped"),
      fullTask("c1", "charlie", "stopped", "crashed"),
      fullTask("a1", "delta", "archived", "stopped"),
    ],
    orphans: [{ id: "task_folder:/p/echo", kind: "task_folder", path: "/p/echo", worktrees: [], bytes: 1_000_000 }],
    clones_bytes: 2_500_000_000,
  };
}
