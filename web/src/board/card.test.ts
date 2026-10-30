import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { agent, schema } from "../test/fixtures";
import { repoSummary, slotOf, stateBadge, useConfirm } from "./card";

const shown = (a: ReturnType<typeof agent>) => stateBadge(schema.card_fields, a)?.text ?? null;

// Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
describe("card rules", () => {
  it("a_card_shows_one_state_badge_stop_cause_first_then_blocked_then_paused", () => {
    expect(shown(agent({ status: "stopped", stop_cause: "crashed", paused: true }))).toBe("Crashed");
    expect(shown(agent({ status: "needs_you", blocked_reason: "permission", paused: true }))).toBe("Permission needed");
    expect(shown(agent({ status: "working", paused: true }))).toBe("Paused");
    expect(shown(agent({ status: "working" }))).toBeNull();
  });

  it("has_changes_is_a_dot_not_a_badge", () => {
    const def = schema.card_fields.find((f) => f.key === "has_changes")!;
    expect(slotOf(def)).toBe("changes");
  });

  it("the_card_names_the_first_repo_and_counts_the_others", () => {
    expect(repoSummary(["core", "portal", "infra"])).toEqual({ first: "core", more: "+2", all: "core\nportal\ninfra" });
    expect(repoSummary(["core"])).toEqual({ first: "core", more: null, all: "core" });
    expect(repoSummary([])).toBeNull();
  });
});

describe("useConfirm", () => {
  const clock = () => {
    let t = 0;
    return { now: () => t, set: (v: number) => (t = v) };
  };

  it("stop_needs_a_second_click_on_the_red_icon", () => {
    const c = clock();
    const { result } = renderHook(() => useConfirm(c.now));
    let run = true;
    act(() => void (run = result.current.press("stop", true)));
    expect(run, "the first click only asks for confirmation").toBe(false);
    expect(result.current.armed("stop")).toBe(true);
    act(() => void (run = result.current.press("stop", true)));
    expect(run).toBe(true);
    expect(result.current.armed("stop"), "confirmation resets after use").toBe(false);
  });

  it("an_armed_stop_disarms_after_a_few_seconds", () => {
    const c = clock();
    const { result, rerender } = renderHook(() => useConfirm(c.now));
    act(() => void result.current.press("stop", true));
    c.set(3.9);
    rerender();
    expect(result.current.armed("stop")).toBe(true);
    c.set(4.01);
    rerender();
    expect(result.current.armed("stop")).toBe(false);
  });

  it("clicking_elsewhere_disarms_stop", () => {
    const { result } = renderHook(() => useConfirm());
    act(() => void result.current.press("stop", true));
    act(() => void document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
    expect(result.current.armed("stop")).toBe(false);
  });

  it("pressing_the_armed_button_again_confirms_rather_than_disarms", () => {
    const { result } = renderHook(() => useConfirm());
    const button = document.body.appendChild(document.createElement("button"));
    button.dataset.confirming = "";
    act(() => void result.current.press("stop", true));
    act(() => void button.dispatchEvent(new Event("pointerdown", { bubbles: true })));
    expect(result.current.armed("stop")).toBe(true);
    button.remove();
  });

  it("plain_actions_run_on_one_click_with_no_input", () => {
    const { result } = renderHook(() => useConfirm());
    let run = false;
    act(() => void (run = result.current.press("pause", false)));
    expect(run).toBe(true);
  });
});
