import { describe, expect, it, vi } from "vitest";
import { FleetClient, FleetError, PLAN_CHANGED } from "./client";
import { agent, schema } from "../test/fixtures";
import { spawnRequest } from "../shell/choices";

function fakeFetch(status: number, body: unknown) {
  return vi.fn(async (_url: string, _init?: RequestInit) => new Response(status === 204 ? null : JSON.stringify(body), { status }));
}

describe("FleetClient", () => {
  it("spawn posts the request it is given", async () => {
    const f = fakeFetch(200, { agent: agent(), warnings: [] });
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    const req = spawnRequest("p1", "me", ["core"], "fix it");
    await c.spawn(req);
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("http://d/api/agents");
    expect(init?.method).toBe("POST");
    expect(JSON.parse(init?.body as string)).toEqual(req);
  });

  it("stop_task_and_restart_post_to_the_task_routes", async () => {
    const f = fakeFetch(200, { stopped: 3 });
    const c = new FleetClient("", f as unknown as typeof fetch);
    expect(await c.stopTask("o/1")).toEqual({ stopped: 3 });
    await c.restartOrchestrator("o/1");
    expect(f.mock.calls.map(([url, init]) => [url, init?.method, init?.body])).toEqual([
      ["/api/tasks/o%2F1/stop", "POST", undefined],
      ["/api/tasks/o%2F1/restart", "POST", undefined],
    ]);
  });

  it("fleet_view_route", async () => {
    const f = fakeFetch(200, { task: "o", breakdowns: [], gate_runs: [] });
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.fleetView("o/1");
    expect(f.mock.calls[0]![0]).toBe("/api/tasks/o%2F1/fleet");
  });

  it("decide_breakdown_posts_the_version", async () => {
    const f = fakeFetch(204, null);
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.decideBreakdown("o", { approve: false, comment: "split it", version: 3 });
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("/api/tasks/o/breakdown/decide");
    expect(JSON.parse(init?.body as string)).toEqual({ approve: false, comment: "split it", version: 3 });
  });

  it("grant_and_refuse_routes", async () => {
    const f = fakeFetch(204, null);
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.grantRepo("a/1", "portal");
    await c.refuseRepo("a/1", "not now");
    expect(f.mock.calls.map(([url, init]) => [url, JSON.parse(init?.body as string)])).toEqual([
      ["/api/agents/a%2F1/repo-request/grant", { actor: "local", repo: "portal" }],
      ["/api/agents/a%2F1/repo-request/refuse", { actor: "local", reason: "not now" }],
    ]);
  });

  it("checkouts_route_encodes_both_segments", async () => {
    const f = fakeFetch(200, { checkouts: [{ path: "/s/x", main: true, branch: "main", commit: "abc", subject: "s" }] });
    const c = new FleetClient("", f as unknown as typeof fetch);
    expect((await c.checkouts("my product", "a/b")).map((x) => x.path)).toEqual(["/s/x"]);
    expect(f.mock.calls[0]![0]).toBe("/api/products/my%20product/repos/a%2Fb/checkouts");
  });

  it("repos_for_a_product", async () => {
    const f = fakeFetch(200, { repos: [] });
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.repos("p 1");
    await c.repos();
    expect(f.mock.calls.map(([u]) => u)).toEqual(["/api/repos?product=p%201", "/api/repos"]);
  });

  it("a_refusal_carries_its_tool", async () => {
    const tool = { kind: "not-installed", tool: "gh", fix: { install: "brew install gh", link: "https://x.invalid", manager: "brew" } };
    const c = new FleetClient("", fakeFetch(424, { error: "gh is missing", tool }) as unknown as typeof fetch);
    await expect(c.spawn({} as never)).rejects.toMatchObject({ message: "gh is missing", tool });
  });

  it("reads the definitions", async () => {
    const f = fakeFetch(200, [{ kind: "orchestrator", name: "lead", summary: "", builtin: true, overridden: false }]);
    const c = new FleetClient("", f as unknown as typeof fetch);
    expect((await c.definitions()).map((d) => d.name)).toEqual(["lead"]);
    expect(f.mock.calls[0]![0]).toBe("/api/definitions");
  });

  it("saves a definition as plain text and resets it", async () => {
    const f = fakeFetch(200, { kind: "specialist", name: "my rust", summary: "", builtin: false, overridden: true });
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.saveDefinition("specialist", "my rust", "---\nname: rust\n---\n");
    const [url, init] = f.mock.calls[0]! as [string, RequestInit];
    expect([url, init.method]).toEqual(["/api/definitions/specialist/my%20rust", "PUT"]);
    expect(init.body).toBe("---\nname: rust\n---\n");
    expect(init.headers).toEqual({ "content-type": "text/plain" });
    await c.resetDefinition("pr_style", "plain");
    expect(f.mock.calls[1]![0]).toBe("/api/definitions/pr_style/plain");
    expect((f.mock.calls[1]![1] as RequestInit).method).toBe("DELETE");
  });

  it("reads the personas and the tasks", async () => {
    const f = fakeFetch(200, { personas: [] });
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.personas();
    await c.tasks();
    expect(f.mock.calls.map((call) => call[0])).toEqual(["/api/personas", "/api/tasks"]);
  });

  it("run sends the request the action definition describes, keeping text literal", async () => {
    const f = fakeFetch(204, null);
    const c = new FleetClient("", f as unknown as typeof fetch);
    const answer = schema.actions.find((a) => a.id === "answer")!;
    await c.run(answer, agent({ id: "x1" }), '$(rm -rf /) "q" -n');
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("/api/agents/x1/answer");
    expect(JSON.parse(init?.body as string)).toEqual({ actor: "local", text: '$(rm -rf /) "q" -n' });
    const stop = schema.actions.find((a) => a.id === "stop")!;
    await c.run(stop, agent({ id: "x1" }));
    expect(f.mock.calls[1]![1]?.method).toBe("POST");
    expect(f.mock.calls[1]![0]).toBe("/api/agents/x1/stop");
  });

  it("turns a non-2xx response into a FleetError carrying the server's message", async () => {
    const c = new FleetClient("", fakeFetch(400, { error: 'unknown task "nope"' }) as unknown as typeof fetch);
    await expect(c.spawn(spawnRequest("p1", "me", [], "x"))).rejects.toThrow(FleetError);
    await expect(c.spawn(spawnRequest("p1", "me", [], "x"))).rejects.toThrow('unknown task "nope"');
  });

  it("reads an agent's diff report from the daemon", async () => {
    const report = { workspaces: [{ repo: "core", diff: "+x\n", truncated: false }] };
    const f = fakeFetch(200, report);
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    expect(await c.diff("a/1")).toEqual(report);
    expect(f.mock.calls[0]![0]).toBe("http://d/api/agents/a%2F1/diff");
  });

  it("reads the ui schema, snapshot and repo list", async () => {
    const c = new FleetClient("", (async (url: string) => new Response(JSON.stringify(url.endsWith("repos") ? { repos: [{ name: "core", source: "path", location: "/r/core" }] } : url.endsWith("schema") ? schema : { seq: 0, agents: [] }))) as unknown as typeof fetch);
    expect(await c.repos()).toEqual([{ name: "core", source: "path", location: "/r/core" }]);
    expect(await c.uiSchema()).toEqual(schema);
    expect(await c.snapshot()).toEqual({ seq: 0, agents: [] });
  });

  it("keeps the field a refusal is about", async () => {
    const f = fakeFetch(400, { error: "bad target", field: "target" });
    const c = new FleetClient("", f as unknown as typeof fetch);
    const err = await c.products().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(FleetError);
    expect((err as FleetError).message).toBe("bad target");
    expect((err as FleetError).field).toBe("target");
  });
});

describe("the tools check", () => {
  it("an_unanswered_tools_check_gives_up_after_fleet_client_s_45_seconds", async () => {
    vi.useFakeTimers();
    try {
      const hang: typeof fetch = (_input, init) =>
        new Promise((_, reject) => init?.signal?.addEventListener("abort", () => reject(new Error("timed out"))));
      const answer = new FleetClient("", hang).tools();
      const settled = expect(answer).rejects.toThrow();
      await vi.advanceTimersByTimeAsync(45_000);
      await settled;
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("the Tasks view's calls", () => {
  it("the_tasks_calls_hit_fleet_clients_paths", async () => {
    const calls: { method: string; url: string; body: unknown }[] = [];
    const record: typeof fetch = async (input, init) => {
      calls.push({ method: init?.method ?? "GET", url: String(input), body: init?.body ? JSON.parse(String(init.body)) : undefined });
      return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
    };
    const c = new FleetClient("http://d", record);
    const id = "a/b c";
    const enc = "a%2Fb%20c";
    const plan = { items: [{ path: "/p", bytes: 1, uncommitted_files: 0, stays: false }], total: 1 };
    await c.refreshTasks();
    await c.archivePlan(id);
    await c.archive(id, plan);
    await c.orphanPlan(id);
    await c.archiveOrphan(id, plan);
    await c.markDone(id);
    await c.setTaskRelease(id, 7);
    await c.setTaskRelease(id, null);
    expect(calls).toEqual([
      { method: "POST", url: "http://d/api/tasks/refresh", body: undefined },
      { method: "GET", url: `http://d/api/tasks/${enc}/archive-plan`, body: undefined },
      { method: "POST", url: `http://d/api/tasks/${enc}/archive`, body: { items: plan.items } },
      { method: "POST", url: "http://d/api/orphans/plan", body: { id } },
      { method: "POST", url: "http://d/api/orphans/archive", body: { id, items: plan.items } },
      { method: "POST", url: `http://d/api/tasks/${enc}/done`, body: undefined },
      { method: "PUT", url: `http://d/api/tasks/${enc}/release`, body: { release: 7 } },
      { method: "PUT", url: `http://d/api/tasks/${enc}/release`, body: { release: null } },
    ]);
    expect(PLAN_CHANGED).toBe("the task changed since you confirmed; review the new plan");
  });

  it("the_release_calls_hit_fleet_clients_paths", async () => {
    const calls: { method: string; url: string; body: unknown }[] = [];
    const record: typeof fetch = async (input, init) => {
      calls.push({ method: init?.method ?? "GET", url: String(input), body: init?.body ? JSON.parse(String(init.body)) : undefined });
      return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
    };
    const c = new FleetClient("http://d", record);
    await c.updateRelease(7, { released: true });
    await c.deleteRelease(7);
    expect(calls).toEqual([
      { method: "PATCH", url: "http://d/api/releases/7", body: { released: true } },
      { method: "DELETE", url: "http://d/api/releases/7", body: undefined },
    ]);
  });
});
