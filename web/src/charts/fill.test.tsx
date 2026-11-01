import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { column, layout, motion } from "../generated/layout";
import { ColumnBars } from "./ColumnBars";
import { Donut } from "./Donut";
import { Meter } from "./Meter";
import { SegmentedBar } from "./SegmentedBar";
import { Sparkline } from "./Sparkline";
import { Stat } from "./Stat";
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

const size = (width: number, height: number) => act(() => observed?.([{ contentRect: { width, height } }]));
const off = () => (document.documentElement.dataset.animations = "off");
const svgH = (c: HTMLElement) => Number(c.querySelector("svg")!.getAttribute("height"));

const bars = [
  { label: "idle", value: 3, paint: { tone: "neutral" as const } },
  { label: "running", value: 5, paint: { tone: "active" as const } },
];
const series = [{ label: "a", paint: { series: 0 as const }, points: [{ t: 0, v: 1 }, { t: 10, v: 4 }] }];

describe("blocks fill their container's height", () => {
  it("column_bars_plot_grows_to_the_height_given_and_keeps_its_minimum_without_one", () => {
    off();
    const { container } = render(<ColumnBars label="C" bars={bars} />);
    size(300, 0);
    expect(svgH(container)).toBe(layout.chartPlot + column.tickLen);
    size(300, 240);
    expect(svgH(container)).toBe(240);
    size(300, 30);
    expect(svgH(container)).toBe(layout.chartPlot + column.tickLen);
    for (let i = 0; i < 5; i++) size(300, 240);
    expect(svgH(container)).toBe(240);
  });

  it("time_series_plot_grows_to_the_height_given_and_keeps_its_minimum_without_one", () => {
    off();
    const { container } = render(<TimeSeries label="T" from={0} to={10} series={series} />);
    size(400, 0);
    const min = svgH(container);
    size(400, min + 100);
    expect(svgH(container)).toBe(min + 100);
    size(400, 10);
    expect(svgH(container)).toBe(min);
    for (let i = 0; i < 5; i++) size(400, min + 100);
    expect(svgH(container)).toBe(min + 100);
  });

  it("sparkline_fills_its_height_and_keeps_its_minimum_without_one", () => {
    off();
    const { container } = render(<Sparkline values={[1, 2, 3]} />);
    size(200, 0);
    expect(svgH(container)).toBe(layout.chartSpark);
    size(200, 120);
    expect(svgH(container)).toBe(120);
    size(200, 120);
    expect(svgH(container)).toBe(120);
  });

  it("donut_grows_to_the_height_given_within_its_width", () => {
    off();
    const slices = [{ label: "a", value: 1, paint: { series: 0 as const } }];
    const two = [...slices, { label: "b", value: 1, paint: { series: 1 as const } }];
    const { container } = render(<Donut label="D" slices={slices} />);
    const min = layout.chartPlot + 2 * column.tickLen;
    size(400, 0);
    expect(svgH(container)).toBe(min);
    size(400, 220);
    expect(svgH(container)).toBe(220);
    expect(Number(container.querySelector("svg")!.getAttribute("width"))).toBe(220);
    size(150, 220);
    expect(svgH(container)).toBe(Math.max(min, 150));
    size(400, 20);
    expect(svgH(container)).toBe(min);
    cleanup();
    const withLegend = render(<Donut label="D" slices={two} />).container;
    size(400, 300);
    expect(svgH(withLegend)).toBe(200);
  });

  it("stat_sparkline_shrinks_to_fit_a_short_cell", () => {
    off();
    const { container } = render(<Stat label="S" value={5} spark={[1, 2, 3]} />);
    size(200, 24);
    expect(svgH(container)).toBe(24);
    size(200, 6);
    expect(svgH(container)).toBe(layout.markerH);
  });
});

describe("column labels", () => {
  it("long_labels_wrap_to_two_lines_and_read_whole_in_the_tooltip_and_table", () => {
    off();
    const long = "running agents";
    const { container } = render(<ColumnBars label="C" bars={[{ ...bars[0]!, label: long }, bars[1]!]} />);
    size(200, 120);
    const el = container.querySelector(".chart-col-label")!;
    expect(el.textContent).toBe(long);
    const rule = readFileSync("src/charts/charts.css", "utf8").match(/\.chart-col-label \{[^}]*\}/)![0];
    expect(rule).toMatch(/-webkit-line-clamp:\s*2/);
    expect(rule).not.toMatch(/nowrap/);
    fireEvent.mouseEnter(container.querySelector(".chart-col-hit")!);
    expect(container.querySelector(".chart-tooltip")!.textContent).toContain(long);
    expect(Array.from(container.querySelectorAll(".chart-hidden th")).map((t) => t.textContent)).toContain(long);
  });
});

describe("Meter tooltip", () => {
  it("follows_live_values_while_hovered", () => {
    off();
    const { container, rerender } = render(<Meter label="B" value={30} max={60} format={(n) => `$${n}`} />);
    size(300, 100);
    fireEvent.mouseEnter(container.querySelector(".chart-seg-hit")!);
    expect(container.querySelector(".chart-tooltip")!.textContent).toContain("$30");
    rerender(<Meter label="B" value={45} max={60} format={(n) => `$${n}`} />);
    expect(container.querySelector(".chart-tooltip")!.textContent).toContain("$45");
    expect(container.querySelector(".chart-tooltip")!.textContent).not.toContain("$30");
  });
});

describe("Stat states", () => {
  it("null_shows_no_data", () => {
    off();
    render(<Stat label="S" value={null} />);
    expect(screen.getByText("No data")).toBeTruthy();
  });

  it("null_then_a_number_plays_the_step_up_once", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<Stat label="S" value={null} />);
    expect(container.querySelector(".chart-stat-value")).toBeNull();
    rerender(<Stat label="S" value={10} />);
    act(() => void vi.advanceTimersByTime(motion.wipe / 2));
    const mid = Number(container.querySelector(".chart-stat-value")!.textContent);
    expect(mid).toBeGreaterThanOrEqual(0);
    expect(mid).toBeLessThan(10);
    act(() => void vi.advanceTimersByTime(5000));
    expect(container.querySelector(".chart-stat-value")!.textContent).toBe("10");
    rerender(<Stat label="S" value={null} />);
    rerender(<Stat label="S" value={20} />);
    expect(container.querySelector(".chart-stat-value")!.textContent).toBe("20");
  });

  it("an_integer_target_never_shows_a_decimal_on_the_way", () => {
    vi.useFakeTimers();
    const { container } = render(<Stat label="S" value={11} />);
    for (let i = 0; i < 40; i++) {
      act(() => void vi.advanceTimersByTime(16));
      expect(container.querySelector(".chart-stat-value")!.textContent).toMatch(/^\d+$/);
    }
  });
});

describe("SegmentedBar with no usable max", () => {
  it("max_zero_with_a_positive_total_scales_to_the_total", () => {
    off();
    const segs = [
      { label: "a", value: 6, paint: { series: 0 as const } },
      { label: "b", value: 4, paint: { series: 1 as const } },
    ];
    const { container } = render(<SegmentedBar label="S" segments={segs} max={0} />);
    size(300, 100);
    const widths = Array.from(container.querySelectorAll(".chart-seg")).map((r) => Number(r.getAttribute("width")));
    expect(widths).toEqual([180, 120]);
    expect(screen.queryByText("No data")).toBeNull();
  });
});
