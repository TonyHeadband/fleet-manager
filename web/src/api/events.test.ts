import { beforeEach, describe, expect, it, vi } from "vitest";
import { subscribe, subscribePane } from "./events";
import { FakeEventSource, agent } from "../test/fixtures";

const factory = (url: string) => new FakeEventSource(url) as unknown as EventSource;

beforeEach(() => FakeEventSource.reset());

describe("subscribe", () => {
  it("subscribe_replaces_state_on_snapshot_event_and_streams_deltas", () => {
    const onSnapshot = vi.fn();
    const onEvent = vi.fn();
    const stop = subscribe("http://d", onSnapshot, onEvent, factory);
    const es = FakeEventSource.for("/api/events");
    expect(es.url).toBe("http://d/api/events");
    es.emit("snapshot", { seq: 3, agents: [agent()] }, "3");
    es.emit("event", { seq: 4, kind: { type: "agent_removed", id: "a1" } }, "4");
    expect(onSnapshot).toHaveBeenCalledWith({ seq: 3, agents: [agent()] });
    expect(onEvent).toHaveBeenCalledWith({ seq: 4, kind: { type: "agent_removed", id: "a1" } });
    stop();
    expect(es.closed).toBe(true);
  });

  it("ignores a malformed payload instead of throwing into the page", () => {
    const onEvent = vi.fn();
    subscribe("", vi.fn(), onEvent, factory);
    const es = FakeEventSource.for("/api/events");
    const bad = new MessageEvent("event", { data: "{not json" });
    // @ts-expect-error reaching into the fake to deliver raw text
    for (const fn of es.handlers.get("event") ?? []) fn(bad);
    expect(onEvent).not.toHaveBeenCalled();
  });
});

describe("subscribePane", () => {
  it("passes pane text through and reports when the stream closes", () => {
    const onText = vi.fn();
    const onClosed = vi.fn();
    subscribePane("", "a/1", onText, onClosed, factory);
    const es = FakeEventSource.for("/terminal");
    expect(es.url).toBe("/api/agents/a%2F1/terminal");
    es.emit("pane", "line one\nline two");
    es.emit("closed", "");
    expect(onText).toHaveBeenCalledWith("line one\nline two");
    expect(onClosed).toHaveBeenCalled();
  });
});
