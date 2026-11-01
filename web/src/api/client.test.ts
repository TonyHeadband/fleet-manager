import { describe, expect, it, vi } from "vitest";
import { FleetClient, FleetError, PLAN_CHANGED, withToken } from "./client";
import { agent, schema } from "../test/fixtures";
import { spawnRequest } from "../shell/choices";

function fakeFetch(status: number, body: unknown) {
  return vi.fn(async (_url: string, _init?: RequestInit) => new Response(status === 204 ? null : JSON.stringify(body), { status }));
}

describe("FleetClient", () => {
  it("the_app_s_token_goes_with_every_call", async () => {
    const f = fakeFetch(200, { agent: agent(), warnings: [] });
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    c.token = "t0k";
    await c.snapshot();
    await c.spawn(spawnRequest("p1", "me", ["core"], "fix it"));
    expect(f.mock.calls.map(([, init]) => init?.headers)).toEqual([
      { "x-fleet-token": "t0k" },
      { "content-type": "application/json", "x-fleet-token": "t0k" },
    ]);
  });

  it("without_a_token_no_header_is_sent", async () => {
    const f = fakeFetch(200, { seq: 0, agents: [] });
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    await c.snapshot();
    expect(f.mock.calls[0]![1]?.headers).toBeUndefined();
  });

  it("a_stream_url_carries_the_token_as_a_query_parameter", () => {
    expect(withToken("http://d/api/events", "t0k")).toBe("http://d/api/events?token=t0k");
    expect(withToken("/api/x?a=1", "t0k")).toBe("/api/x?a=1&token=t0k");
    expect(withToken("http://d/api/events", null)).toBe("http://d/api/events");
    expect(withToken("http://d/api/events", undefined)).toBe("http://d/api/events");
  });

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

  it("writes products and their repos, settings and internal resources at the daemon's routes", async () => {
    const f = fakeFetch(200, { commits: [] });
    const c = new FleetClient("", f as unknown as typeof fetch);
    await c.createProduct({ name: "Core", default_persona: null, internal_url: null });
    await c.updateProduct("p 1", { name: "Alpha" });
    await c.upsertRepo("p 1", { name: "docs", path: "/src/docs", url: null });
    await c.deleteRepo("p 1", "my repo");
    await c.deleteProduct("p 1");
    await c.putHiddenSpecialists("p 1", ["rust"]);
    await c.putOverrides("p 1", [{ section: "limits", key: "max_steps", value: "30" }]);
    await c.effectiveSettings();
    await c.internalBranches("p 1");
    await c.internalFile("p 1", "task/a b", "designs/a b.md");
    await c.internalHistory("p 1", "main", "a.md");
    const calls = f.mock.calls.map(([url, init]) => [(init as RequestInit | undefined)?.method ?? "GET", url, (init as RequestInit | undefined)?.body]);
    expect(calls).toEqual([
      ["POST", "/api/products", JSON.stringify({ name: "Core", default_persona: null, internal_url: null })],
      ["PATCH", "/api/products/p%201", JSON.stringify({ name: "Alpha" })],
      ["PUT", "/api/products/p%201/repos", JSON.stringify({ name: "docs", path: "/src/docs", url: null })],
      ["DELETE", "/api/products/p%201/repos/my%20repo", undefined],
      ["DELETE", "/api/products/p%201", undefined],
      ["PUT", "/api/products/p%201/hidden-specialists", JSON.stringify({ hidden: ["rust"] })],
      ["PUT", "/api/products/p%201/overrides", JSON.stringify({ overrides: [{ section: "limits", key: "max_steps", value: "30" }] })],
      ["GET", "/api/settings/effective", undefined],
      ["GET", "/api/products/p%201/internal/branches", undefined],
      ["GET", "/api/products/p%201/internal/file?branch=task%2Fa%20b&path=designs%2Fa%20b.md", undefined],
      ["GET", "/api/products/p%201/internal/history?branch=main&path=a.md", undefined],
    ]);
  });

  it("answers the internal files and history as egui's client does", async () => {
    const files = new FleetClient("", fakeFetch(200, { branch: "main", files: ["a.md"] }) as unknown as typeof fetch);
    expect(await files.internalFiles("p", "main")).toEqual(["a.md"]);
    const file = new FleetClient("", fakeFetch(200, { branch: "main", path: "a.md", text: "# A" }) as unknown as typeof fetch);
    expect(await file.internalFile("p", "main", "a.md")).toBe("# A");
    const commit = { sha: "s", at: 1, author: "o", subject: "Add" };
    const history = new FleetClient("", fakeFetch(200, { commits: [commit] }) as unknown as typeof fetch);
    expect(await history.internalHistory("p", "main", "a.md")).toEqual([commit]);
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

describe("request timeouts, as fleet-client's", () => {
  const hang: typeof fetch = (_input, init) =>
    new Promise((_, reject) => init?.signal?.addEventListener("abort", () => reject(init.signal!.reason)));

  /** Whether the call has settled `ms` after it was sent. */
  async function settledAfter(call: (c: FleetClient) => Promise<unknown>, ms: number) {
    vi.useFakeTimers();
    try {
      let settled = false;
      const answer = call(new FleetClient("", hang)).catch((e: unknown) => e);
      void answer.then(() => (settled = true));
      await vi.advanceTimersByTimeAsync(ms);
      return { settled, error: settled ? await answer : undefined };
    } finally {
      vi.useRealTimers();
    }
  }

  it("an_unanswered_call_gives_up_after_fleet_client_s_10_seconds", async () => {
    expect((await settledAfter((c) => c.products(), 9_999)).settled).toBe(false);
    const { settled, error } = await settledAfter((c) => c.products(), 10_000);
    expect(settled).toBe(true);
    expect(error).toBeInstanceOf(FleetError);
    expect((error as FleetError).message).toBe("the daemon did not answer in time");
  });

  it("a_persona_check_and_a_restart_wait_fleet_client_s_25_seconds", async () => {
    for (const call of [(c: FleetClient) => c.checkPersona("me"), (c: FleetClient) => c.restartOrchestrator("o")]) {
      expect((await settledAfter(call, 24_999)).settled).toBe(false);
      expect((await settledAfter(call, 25_000)).settled).toBe(true);
    }
  });

  it("a_tools_recheck_waits_fleet_client_s_45_seconds", async () => {
    expect((await settledAfter((c) => c.recheckTools(), 44_999)).settled).toBe(false);
    expect((await settledAfter((c) => c.recheckTools(), 45_000)).settled).toBe(true);
  });

  it("calls_that_may_clone_or_weigh_folders_wait_fleet_client_s_30_minutes", async () => {
    const plan = { items: [], total: 0 };
    const long: Array<(c: FleetClient) => Promise<unknown>> = [
      (c) => c.spawn({} as never),
      (c) => c.grantRepo("a", "core"),
      (c) => c.createProduct({} as never),
      (c) => c.updateProduct("p", {}),
      (c) => c.archivePlan("t"),
      (c) => c.archive("t", plan),
      (c) => c.orphanPlan("t"),
      (c) => c.archiveOrphan("t", plan),
    ];
    for (const call of long) {
      expect((await settledAfter(call, 30 * 60_000 - 1)).settled).toBe(false);
      expect((await settledAfter(call, 30 * 60_000)).settled).toBe(true);
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

  it("chat_history_asks_for_the_page_before_a_seq", async () => {
    const page = { items: [], earlier: false };
    const f = fakeFetch(200, page);
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    expect(await c.chatHistory("a/1", 40, 25)).toEqual(page);
    expect(f.mock.calls[0]![0]).toBe("http://d/api/agents/a%2F1/chat/history?before=40&limit=25");
    expect(f.mock.calls[0]![1]?.method).toBe("GET");
  });

  it("answer_ask_posts_the_answer_and_rejects_with_the_daemon_message", async () => {
    const f = fakeFetch(204, null);
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    await c.answerAsk("a1", "k/1", { choice: "0.1", text: null });
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("http://d/api/agents/a1/asks/k%2F1");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe(JSON.stringify({ choice: "0.1", text: null }));
    const g = fakeFetch(409, { error: "already answered" });
    await expect(new FleetClient("http://d", g as unknown as typeof fetch).answerAsk("a1", "k", { choice: "", text: "x" })).rejects.toThrow("already answered");
  });
});
