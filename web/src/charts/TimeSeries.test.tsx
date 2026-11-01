import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { layout, motion } from "../generated/layout";
import type { Paint } from "./paint";
import { TimeSeries } from "./TimeSeries";

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
const cells = () => Array.from(document.querySelectorAll(".chart-hidden td")).map((c) => c.textContent);
const noNaN = (c: HTMLElement) => expect(c.innerHTML).not.toMatch(/NaN|Infinity/);
const pts = (el: Element) =>
  (el.getAttribute("points") ?? "").split(" ").map((p) => p.split(",").map(Number) as [number, number]);

const X0 = layout.chartValueW;
const W = 300;
const PW = W - X0;
const pt = (t: number, v: number | null) => ({ t, v });
const a = { label: "input", paint: { series: 0 as const }, points: [pt(0, 200), pt(10, 100), pt(20, 50)] };
const b = { label: "output", paint: { series: 1 as const }, points: [pt(0, 50), pt(10, 25), pt(20, 0)] };

describe("TimeSeries", () => {
  it("lines_share_one_y_axis_from_zero", () => {
    motionOff();
    const { container } = render(<TimeSeries label="Tokens" from={0} to={20} series={[a, b]} />);
    size(W);
    const lines = container.querySelectorAll(".chart-ts-line");
    expect(lines).toHaveLength(2);
    expect(lines[0]!.getAttribute("stroke")).toBe("var(--series-0)");
    const [a0, a1] = [pts(lines[0]!)[0]!, pts(lines[0]!)[1]!];
    const b0 = pts(lines[1]!)[0]!;
    const baseline = layout.chartPlot;
    expect(a0[1]).toBeCloseTo(0);
    expect(a1[1]).toBeCloseTo(baseline / 2);
    expect(b0[1]).toBeCloseTo(baseline * 0.75);
    expect(pts(lines[1]!)[2]![1]).toBeCloseTo(baseline);
    expect(Array.from(container.querySelectorAll(".chart-ytick")).map((t) => t.textContent)).toEqual(["0", "50", "100", "150", "200"]);
    expect(container.querySelectorAll(".chart-grid").length).toBe(4);
    expect(pts(lines[0]!)[2]![0]).toBeCloseTo(PW);
    noNaN(container);
  });

  it("a_null_breaks_the_line", () => {
    motionOff();
    const s = { label: "x", paint: { series: 0 as const }, points: [pt(0, 1), pt(5, 2), pt(10, null), pt(15, 4), pt(20, 5)] };
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[s]} />);
    size(W);
    const lines = container.querySelectorAll(".chart-ts-line");
    expect(lines).toHaveLength(2);
    expect(pts(lines[0]!)).toHaveLength(2);
    expect(pts(lines[1]!)).toHaveLength(2);
    expect(cells()).toEqual(["1", "2", "—", "4", "5"]);
  });

  it("a_one_point_series_draws_a_marker", () => {
    motionOff();
    const one = { label: "x", paint: { series: 0 as const }, points: [pt(10, 3)] };
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[one]} />);
    size(W);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(0);
    expect(container.querySelectorAll(".chart-marker")).toHaveLength(1);
    expect(container.querySelector(".chart-nodata")).toBeNull();
  });

  it("an_isolated_point_between_nulls_draws_a_marker", () => {
    motionOff();
    const s = { label: "x", paint: { series: 0 as const }, points: [pt(0, null), pt(10, 3), pt(20, null)] };
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[s]} />);
    size(W);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(0);
    expect(container.querySelectorAll(".chart-marker")).toHaveLength(1);
  });

  it("the_crosshair_snaps_to_each_series_nearest_point_at_or_before", () => {
    motionOff();
    const gap = { label: "output", paint: { series: 1 as const }, points: [pt(0, 5), pt(8, null), pt(20, 7)] };
    const { container } = render(
      <TimeSeries label="T" from={0} to={20} series={[a, gap]} formatX={(t) => `T${t}`} formatY={(n) => `${n}u`} />,
    );
    size(W);
    const hit = container.querySelector(".chart-ts-hit")!;
    expect(container.querySelector(".chart-cross")).toBeNull();
    fireEvent.mouseMove(hit, { clientX: X0 + PW / 2 });
    expect(Number(container.querySelector(".chart-cross")!.getAttribute("x1"))).toBeCloseTo(PW / 2);
    const tip = screen.getByRole("tooltip");
    expect(Array.from(tip.children).map((c) => c.textContent)).toEqual(["T10", "input: 100u", "output: —"]);
    fireEvent.mouseMove(hit, { clientX: X0 + PW });
    expect(Array.from(screen.getByRole("tooltip").children).map((c) => c.textContent)).toEqual(["T20", "input: 50u", "output: 7u"]);
    fireEvent.mouseLeave(hit);
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(container.querySelector(".chart-cross")).toBeNull();
  });

  it("the_tooltip_opens_below_the_plot", () => {
    motionOff();
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[a]} />);
    size(W);
    fireEvent.mouseMove(container.querySelector(".chart-ts-hit")!, { clientX: X0 + 10 });
    const top = parseFloat(screen.getByRole("tooltip").style.top);
    const baseline = layout.chartPlot;
    expect(top).toBeGreaterThan(baseline);
  });

  it("points_outside_the_range_are_clipped", () => {
    motionOff();
    const s = { label: "x", paint: { series: 0 as const }, points: [pt(50, 0), pt(150, 100), pt(250, 0)] };
    const out = { label: "y", paint: { series: 1 as const }, points: [pt(300, 5)] };
    const { container } = render(<TimeSeries label="T" from={100} to={200} series={[s, out]} />);
    size(W);
    const lines = container.querySelectorAll(".chart-ts-line");
    expect(lines).toHaveLength(1);
    const p = pts(lines[0]!);
    expect(p).toHaveLength(3);
    expect(p[0]![0]).toBeCloseTo(0);
    expect(p[2]![0]).toBeCloseTo(PW);
    expect(p[0]![1]).toBeCloseTo(p[2]![1]);
    expect(container.querySelectorAll(".chart-marker")).toHaveLength(0);
    expect(cells()).toHaveLength(4);
    noNaN(container);
  });

  it("lines_draw_left_to_right_once", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<TimeSeries label="T" from={0} to={20} series={[a, b]} />);
    size(W);
    const entering = () => Array.from(container.querySelectorAll(".chart-ts-line")).map((r) => r.getAttribute("data-enter"));
    expect(entering()).toEqual(["true", "true"]);
    expect((container.querySelectorAll(".chart-ts-line")[1] as SVGElement).style.getPropertyValue("--i")).toBe("1");
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(entering()).toEqual([null, null]);
    rerender(<TimeSeries label="T" from={0} to={20} series={[{ ...a, points: [...a.points, pt(21, 9)] }, b]} />);
    expect(entering()).toEqual([null, null]);
  });

  it("time_series_survives_all_zero", () => {
    motionOff();
    const z = (s: { label: string; paint: Paint; points: { t: number; v: number | null }[] }) => ({ ...s, points: s.points.map((p) => pt(p.t, 0)) });
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[z(a), z(b)]} />);
    size(W);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(2);
    fireEvent.mouseMove(container.querySelector(".chart-ts-hit")!, { clientX: X0 + 10 });
    noNaN(container);
  });

  it("time_series_draws_nothing_at_width_zero_and_says_no_data_when_empty", () => {
    motionOff();
    const { container, rerender } = render(<TimeSeries label="T" from={0} to={20} series={[a]} />);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(0);
    noNaN(container);
    size(W);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(1);
    rerender(<TimeSeries label="T" from={0} to={20} series={[]} />);
    expect(screen.getByText("No data")).toBeTruthy();
  });

  it("the_legend_shows_for_two_series_and_moves_to_the_table_when_compact", () => {
    motionOff();
    const { container, rerender } = render(<TimeSeries label="T" from={0} to={20} series={[a, b]} />);
    size(W);
    expect(container.querySelectorAll(".chart-legend li")).toHaveLength(2);
    size(layout.chartCompact - 1);
    expect(container.querySelector(".chart-legend")).toBeNull();
    expect(container.querySelectorAll(".chart-hidden tr").length).toBe(6);
    size(W);
    rerender(<TimeSeries label="T" from={0} to={20} series={[a]} />);
    expect(container.querySelector(".chart-legend")).toBeNull();
  });

  it("tick_labels_go_when_tiny", () => {
    motionOff();
    const { container } = render(<TimeSeries label="T" from={0} to={20} series={[a]} />);
    size(layout.chartTiny - 1);
    expect(container.querySelectorAll(".chart-ts-line")).toHaveLength(1);
    expect(container.querySelector(".chart-baseline")).toBeTruthy();
    expect(container.querySelectorAll(".chart-tick-label, .chart-ytick")).toHaveLength(0);
    size(W);
    expect(container.querySelectorAll(".chart-tick-label").length).toBeGreaterThan(0);
    expect(container.querySelectorAll(".chart-ytick").length).toBeGreaterThan(0);
  });

  it("the_default_time_label_is_short_local_hours_and_minutes", () => {
    motionOff();
    const t = new Date(2026, 10, 1, 9, 5).getTime() / 1000;
    const { container } = render(<TimeSeries label="T" from={t} to={t + 3600} series={[{ label: "x", paint: { series: 0 }, points: [pt(t, 1), pt(t + 3600, 2)] }]} />);
    size(W);
    expect(container.querySelector(".chart-scale .chart-tick-label")!.textContent).toBe("09:05");
  });
});
