import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { FleetView } from "../api/generated/FleetView";
import { fakeClient } from "../test/fixtures";
import { useBreakdownDecisions } from "./decisions";

const proposed = (): FleetView => ({ task: "o", breakdowns: [{ version: 2, steps: [], approval: "proposed", comment: null, proposed_at: 0 }], gate_runs: [] });

describe("useBreakdownDecisions, as Lite's breakdown_decided", () => {
  it("the_controls_wait_for_a_view_that_lands_after_the_decision", async () => {
    let answer!: () => void;
    const client = fakeClient({ decideBreakdown: vi.fn(() => new Promise<void>((r) => (answer = r))) });
    const fetch = vi.fn();
    const { result, rerender } = renderHook(({ views }) => useBreakdownDecisions(client, views, fetch, vi.fn()), { initialProps: { views: { o: { view: proposed() } } } });
    act(() => result.current.decide("o", null));
    // A view asked for before the decision lands while it is on its way: it still shows the version proposed.
    rerender({ views: { o: { view: proposed() } } });
    await act(async () => answer());
    expect(result.current.busy("o"), "the view on screen predates the decision").toBe(true);
    rerender({ views: { o: { view: proposed() } } });
    expect(result.current.busy("o"), "a view fetched after it lifts the wait").toBe(false);
  });
});
