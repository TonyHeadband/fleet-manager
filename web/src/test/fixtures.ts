import { vi } from "vitest";
import type { FleetClient } from "../api/client";
import type { FormDef } from "../api/generated/FormDef";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import type { StopCause } from "../api/generated/StopCause";
import type { Task } from "../api/generated/Task";
import type { TaskState } from "../api/generated/TaskState";
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
    workspaces: [{ repo: "core", branch: "fleet/fix-the-tests", worktree: "/w/fix-the-tests/core", base: "abc123", has_changes: false, owned: false }],
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
  return { product: "p1", name, path: null, url: `https://example.test/${name}.git`, workflow: { state: "missing", base: "main" }, env: [], ...over };
}

export const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;

export function fakeClient(over: Partial<Record<keyof FleetClient, unknown>> = {}): FleetClient {
  return {
    baseUrl: "",
    uiSchema: vi.fn().mockResolvedValue(schema),
    repos: vi.fn().mockResolvedValue([repoInfo("core"), repoInfo("portal")]),
    personas: vi.fn().mockResolvedValue({ personas: [] }),
    products: vi.fn().mockResolvedValue({ products: [] }),
    tasks: vi.fn().mockResolvedValue({ tasks: [], orphans: [], clones_bytes: null }),
    spawn: vi.fn().mockResolvedValue({ agent: agent(), warnings: [] }),
    run: vi.fn().mockResolvedValue(undefined),
    diff: vi.fn().mockResolvedValue({ workspaces: [] }),
    agentConfig: vi.fn().mockResolvedValue({ instructions: "", settings: { allow: [], deny: [], env: {} } }),
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
