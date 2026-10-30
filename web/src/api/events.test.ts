import { beforeEach, describe, expect, it, vi } from "vitest";
import { SILENCE_MS, subscribe, subscribePane } from "./events";
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

describe("subscribe watchdog", () => {
  it("reopens a stream that goes silent and reports it disconnected meanwhile", () => {
    vi.useFakeTimers();
    try {
      const onConnection = vi.fn();
      const stop = subscribe("", vi.fn(), vi.fn(), factory, onConnection);
      const first = FakeEventSource.for("/api/events");
      vi.advanceTimersByTime(SILENCE_MS + 5_000);
      expect(first.closed).toBe(true);
      expect(onConnection).toHaveBeenCalledWith(false);
      expect(FakeEventSource.instances).toHaveLength(2);
      stop();
      expect(FakeEventSource.instances[1]!.closed).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a_stream_that_comes_back_reports_a_reconnect", () => {
    const onReconnect = vi.fn();
    subscribe("", vi.fn(), vi.fn(), factory, undefined, onReconnect);
    const es = FakeEventSource.for("/api/events");
    es.emit("open", null);
    expect(onReconnect, "the first open is not a reconnect").not.toHaveBeenCalled();
    es.emit("error", null);
    es.emit("open", null);
    expect(onReconnect).toHaveBeenCalledTimes(1);
  });

  it("keeps a stream that pings", () => {
    vi.useFakeTimers();
    try {
      const stop = subscribe("", vi.fn(), vi.fn(), factory);
      const es = FakeEventSource.for("/api/events");
      for (let t = 0; t < 4 * SILENCE_MS; t += 15_000) {
        vi.advanceTimersByTime(15_000);
        es.emit("ping", "");
      }
      expect(es.closed).toBe(false);
      expect(FakeEventSource.instances).toHaveLength(1);
      stop();
    } finally {
      vi.useRealTimers();
    }
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
