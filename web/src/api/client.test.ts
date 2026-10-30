import { describe, expect, it, vi } from "vitest";
import { FleetClient, FleetError } from "./client";
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
