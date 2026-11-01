import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { layout, motion, space } from "../generated/layout";
import { StatusTimeline } from "./StatusTimeline";

let observed: ((entries: { contentRect: { width: number; height: number } }[]) => void) | null = null;

beforeEach(() => {
  observed = null;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(cb: typeof observed) {
        observed = cb;
      }
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete document.documentElement.dataset.animations;
});

const size = (width: number) => act(() => observed?.([{ contentRect: { width, height: 100 } }]));
const motionOff = () => (document.documentElement.dataset.animations = "off");
const noNaN = (c: HTMLElement) => expect(c.innerHTML).not.toMatch(/NaN|Infinity/);
const px = (v: string) => Number.parseFloat(v);

const W = 300;
const TW = W - layout.chartLabelW - space.buttonGap;
const fmt = (t: number) => `t${t}`;
const lanes = [
  { label: "alpha", bands: [{ from: 0, to: 50, tone: "active" as const, label: "working" }, { from: 50, to: 100, tone: "attention" as const, label: "needs you" }] },
  { label: "beta", bands: [{ from: 25, to: null, tone: "done" as const, label: "review" }] },
];
const band = (c: HTMLElement) => Array.from(c.querySelectorAll<HTMLElement>(".chart-tl-band"));

describe("StatusTimeline", () => {
  it("one_lane_per_entity_with_its_bands_in_status_tones", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="Agents" from={0} to={100} lanes={lanes} formatX={fmt} />);
    size(W);
    expect(container.querySelectorAll(".chart-tl-lane")).toHaveLength(2);
    expect(Array.from(container.querySelectorAll(".chart-tl-label")).map((l) => l.textContent)).toEqual(["alpha", "beta"]);
    const fills = Array.from(container.querySelectorAll<HTMLElement>(".chart-tl-fill")).map((f) => f.style.background);
    expect(fills).toEqual(["var(--tone-active)", "var(--tone-attention)", "var(--tone-done)"]);
    expect(px(band(container)[0]!.style.width)).toBeCloseTo(TW / 2);
    expect(px(band(container)[1]!.style.left)).toBeCloseTo(TW / 2);
    expect(container.querySelector(".chart-baseline")).toBeTruthy();
    expect(screen.getAllByRole("rowheader").map((r) => r.textContent)).toEqual(["alpha · working", "alpha · needs you", "beta · review"]);
    expect(screen.getAllByRole("cell")[2]!.textContent).toBe("t25 – now");
    expect(screen.getByRole("group", { name: "Agents" })).toBeTruthy();
    noNaN(container);
  });

  it("an_open_band_runs_to_the_range_end", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={lanes} />);
    size(W);
    const open = band(container)[2]!;
    expect(px(open.style.left) + px(open.style.width)).toBeCloseTo(TW);
  });

  it("bands_outside_the_range_are_clipped", () => {
    motionOff();
    const l = [{ label: "x", bands: [
      { from: -50, to: 20, tone: "active" as const, label: "a" },
      { from: 80, to: 200, tone: "done" as const, label: "b" },
      { from: -30, to: -10, tone: "caution" as const, label: "gone" },
      { from: 150, to: 160, tone: "caution" as const, label: "gone" },
    ] }];
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={l} />);
    size(W);
    const b = band(container);
    expect(b).toHaveLength(2);
    expect(px(b[0]!.style.left)).toBeCloseTo(0);
    expect(px(b[0]!.style.width)).toBeCloseTo(TW * 0.2);
    expect(px(b[1]!.style.left) + px(b[1]!.style.width)).toBeCloseTo(TW);
  });

  it("hovering_a_band_shows_its_state_and_times", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={lanes} formatX={fmt} />);
    size(W);
    fireEvent.mouseEnter(band(container)[1]!);
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("alpha");
    expect(tip.textContent).toContain("needs you");
    expect(tip.textContent).toContain("t50 – t100");
    fireEvent.mouseLeave(band(container)[1]!);
    expect(screen.queryByRole("tooltip")).toBeNull();
    fireEvent.mouseEnter(band(container)[2]!);
    expect(screen.getByRole("tooltip").textContent).toContain("t25 – now");
  });

  it("bands_wipe_in_once", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<StatusTimeline label="A" from={0} to={100} lanes={lanes} />);
    size(W);
    const entering = () => Array.from(container.querySelectorAll(".chart-tl-fill")).map((r) => r.getAttribute("data-enter"));
    expect(entering()).toEqual(["true", "true", "true"]);
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(entering()).toEqual([null, null, null]);
    rerender(<StatusTimeline label="A" from={0} to={110} lanes={lanes} />);
    expect(entering()).toEqual([null, null, null]);
  });

  it("status_timeline_survives_no_lanes", () => {
    motionOff();
    const { container, rerender } = render(<StatusTimeline label="A" from={0} to={100} lanes={[]} />);
    expect(screen.getByText("No data")).toBeTruthy();
    size(W);
    noNaN(container);
    rerender(<StatusTimeline label="A" from={5} to={5} lanes={[{ label: "e", bands: [{ from: 0, to: null, tone: "active", label: "w" }] }]} />);
    noNaN(container);
  });

  it("a_lane_without_visible_bands_keeps_its_label_and_track", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={[{ label: "idle", bands: [{ from: 200, to: 300, tone: "active", label: "w" }] }]} />);
    size(W);
    expect(container.querySelectorAll(".chart-tl-lane")).toHaveLength(1);
    expect(band(container)).toHaveLength(0);
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("the_legend_lists_distinct_states_and_moves_to_the_table_when_compact", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={lanes} />);
    size(W);
    expect(Array.from(container.querySelectorAll(".chart-legend li")).map((l) => l.textContent)).toEqual(["working", "needs you", "review"]);
    size(layout.chartCompact - 1);
    expect(container.querySelector(".chart-legend")).toBeNull();
    expect(screen.getAllByRole("rowheader")).toHaveLength(3);
  });

  it("tick_labels_go_when_tiny", () => {
    motionOff();
    const { container } = render(<StatusTimeline label="A" from={0} to={100} lanes={lanes} />);
    size(W);
    expect(container.querySelectorAll(".chart-tick-label").length).toBeGreaterThan(0);
    size(layout.chartTiny - 1);
    expect(container.querySelectorAll(".chart-tick-label")).toHaveLength(0);
  });
});
