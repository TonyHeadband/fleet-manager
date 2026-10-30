import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { FleetClient } from "../api/client";
import type { FleetView } from "../api/generated/FleetView";
import { agent } from "../test/fixtures";
import { fleetPanel, useFleetViews } from "./fleetViews";

const view = (task: string, pr?: string): FleetView => ({ task, breakdowns: [], gate_runs: pr ? [{ at: 0, base: "b", steps: [], unmatched: [], pr_url: pr } as never] : [] });

/** A client whose fleet views answer when the test says so. */
function pending() {
  const answers: { resolve: (v: FleetView) => void; reject: (e: Error) => void }[] = [];
  const fleetView = vi.fn(() => new Promise<FleetView>((resolve, reject) => answers.push({ resolve, reject })));
  return { client: { fleetView } as unknown as FleetClient, fleetView, answers };
}

describe("useFleetViews", () => {
  it("an_older_view_never_lands_last", async () => {
    const { client, fleetView, answers } = pending();
    const { result } = renderHook(() => useFleetViews(client, vi.fn()));
    act(() => result.current.fetch("o"));
    act(() => result.current.fetch("o"));
    expect(fleetView, "one request at a time").toHaveBeenCalledTimes(1);
    await act(async () => answers[0]!.resolve(view("o")));
    expect(fleetView, "the one asked for meanwhile follows").toHaveBeenCalledTimes(2);
    await act(async () => answers[1]!.resolve(view("o", "https://pr/2")));
    expect(result.current.views.o?.view).toEqual(view("o", "https://pr/2"));
    expect(result.current.prs).toEqual({ o: true });
  });

  it("a_failed_refresh_keeps_the_view_and_warns", async () => {
    const { client, answers } = pending();
    const say = vi.fn();
    const { result } = renderHook(() => useFleetViews(client, say));
    act(() => result.current.fetch("o"));
    await act(async () => answers[0]!.resolve(view("o")));
    act(() => result.current.fetch("o"));
    await act(async () => answers[1]!.reject(new Error("store is busy")));
    expect(result.current.views.o).toEqual({ view: view("o") });
    expect(say).toHaveBeenCalledWith("warning", "Could not refresh the breakdown: store is busy");
  });

  it("a_first_failure_is_shown_in_its_place", async () => {
    const { client, answers } = pending();
    const say = vi.fn();
    const { result } = renderHook(() => useFleetViews(client, say));
    act(() => result.current.fetch("o"));
    await act(async () => answers[0]!.reject(new Error("no such task")));
    expect(result.current.views.o).toEqual({ error: "no such task" });
    expect(say).not.toHaveBeenCalled();
  });
});

describe("fleetPanel", () => {
  it("a_specialist_belongs_to_its_live_lead", () => {
    const o = agent({ id: "o", kind: "orchestrator", goal: "Fix login", status: "stopped" });
    const o2 = agent({ id: "o2", kind: "orchestrator", task: "o", goal: "Fix login again" });
    const f = agent({ id: "f", kind: "specialist", task: "o" });
    expect(fleetPanel([o, o2, f], f)).toEqual({ type: "member", task: "Fix login again", lead: "o2" });
    expect(fleetPanel([o, o2, f], o2)).toEqual({ type: "task" });
    expect(fleetPanel([agent({ id: "s" })], agent({ id: "s" }))).toBeNull();
    expect(fleetPanel([f], f), "its own lead, with no orchestrator left").toBeNull();
  });
});
