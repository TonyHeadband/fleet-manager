import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TASKS_EVERY_MS, useLoad } from "./hooks";

/** A fetch whose answers the test hands out, in any order. */
function pending<T>() {
  const answers: Array<{ resolve: (v: T) => void; reject: (e: Error) => void }> = [];
  const fetch = vi.fn(() => new Promise<T>((resolve, reject) => answers.push({ resolve, reject })));
  return { fetch, answers };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("useLoad, as Lite's fetch_tasks", () => {
  it("a_stalled_daemon_gets_one_request_at_a_time", async () => {
    const { fetch, answers } = pending<number>();
    const { result } = renderHook(() => useLoad(fetch, true));
    act(() => vi.advanceTimersByTime(TASKS_EVERY_MS * 5));
    expect(fetch, "the ticks wait on the request out").toHaveBeenCalledTimes(1);
    await act(async () => answers[0]!.resolve(1));
    expect(result.current[0]).toEqual({ state: "ready", value: 1 });
    expect(fetch, "one more for all the ticks missed").toHaveBeenCalledTimes(2);
  });

  it("an_older_answer_never_overwrites_a_newer_one", async () => {
    const { fetch, answers } = pending<string>();
    const { result, rerender } = renderHook(({ f }) => useLoad(f), { initialProps: { f: fetch } });
    // Another daemon (a new client): its request goes out while the old one is still out.
    const next = pending<string>();
    rerender({ f: next.fetch });
    await act(async () => next.answers[0]!.resolve("new"));
    await act(async () => answers[0]!.resolve("old"));
    expect(result.current[0]).toEqual({ state: "ready", value: "new" });
  });

  it("a_loaded_list_stops_asking", async () => {
    const fetch = vi.fn().mockResolvedValue(["a"]);
    renderHook(() => useLoad(fetch));
    await act(async () => {});
    act(() => vi.advanceTimersByTime(TASKS_EVERY_MS * 3));
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount(), "no timer left running").toBe(0);
  });

  it("a_failed_load_asks_again_and_a_failed_refresh_keeps_the_list", async () => {
    const fetch = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValueOnce(["a"]).mockRejectedValueOnce(new Error("down"));
    const { result } = renderHook(() => useLoad(fetch));
    await act(async () => {});
    expect(result.current[0]).toEqual({ state: "failed", error: "down" });
    await act(async () => vi.advanceTimersByTime(TASKS_EVERY_MS));
    expect(result.current[0]).toEqual({ state: "ready", value: ["a"] });
    await act(async () => result.current[1]());
    expect(result.current[0]).toEqual({ state: "ready", value: ["a"] });
  });

  it("reload_asks_again_once", async () => {
    const fetch = vi.fn().mockResolvedValueOnce(["a"]).mockResolvedValueOnce(["a", "b"]);
    const { result } = renderHook(() => useLoad(fetch));
    await act(async () => {});
    await act(async () => result.current[1]());
    expect(result.current[0]).toEqual({ state: "ready", value: ["a", "b"] });
    act(() => vi.advanceTimersByTime(TASKS_EVERY_MS * 3));
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});
