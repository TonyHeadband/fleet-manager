import { describe, expect, it, vi } from "vitest";
import { FleetClient, FleetError } from "./client";
import { agent, schema } from "../test/fixtures";

function fakeFetch(status: number, body: unknown) {
  return vi.fn(async (_url: string, _init?: RequestInit) => new Response(status === 204 ? null : JSON.stringify(body), { status }));
}

describe("FleetClient", () => {
  it("spawn posts the project and prompt with the local actor", async () => {
    const f = fakeFetch(200, agent());
    const c = new FleetClient("http://d", f as unknown as typeof fetch);
    await c.spawn("core", "fix it");
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("http://d/api/agents");
    expect(init?.method).toBe("POST");
    expect(JSON.parse(init?.body as string)).toEqual({ actor: "local", repos: ["core"], prompt: "fix it", task_slug: null });
  });

  it("run sends the request the action definition describes, keeping text literal", async () => {
    const f = fakeFetch(204, null);
    const c = new FleetClient("", f as unknown as typeof fetch);
    const answer = schema.actions.find((a) => a.id === "answer")!;
    await c.run(answer, agent({ id: "x1" }), '$(rm -rf /) "q" -n');
    const [url, init] = f.mock.calls[0]!;
    expect(url).toBe("/api/agents/x1/answer");
    expect(JSON.parse(init?.body as string)).toEqual({ actor: "local", text: '$(rm -rf /) "q" -n' });
    const kill = schema.actions.find((a) => a.id === "kill")!;
    await c.run(kill, agent({ id: "x1" }));
    expect(f.mock.calls[1]![1]?.method).toBe("DELETE");
    expect(f.mock.calls[1]![1]?.body).toBeUndefined();
  });

  it("turns a non-2xx response into a FleetError carrying the server's message", async () => {
    const c = new FleetClient("", fakeFetch(400, { error: 'unknown project "nope"' }) as unknown as typeof fetch);
    await expect(c.spawn("nope", "x")).rejects.toThrow(FleetError);
    await expect(c.spawn("nope", "x")).rejects.toThrow('unknown project "nope"');
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
});
