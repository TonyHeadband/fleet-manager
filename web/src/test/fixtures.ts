import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import defaultSchema from "../api/generated/default-ui-schema.json";

export const schema = defaultSchema as UiSchema;

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
