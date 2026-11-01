import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { layout, motion } from "../generated/layout";
import { Donut } from "./Donut";
import { Meter } from "./Meter";
import { SegmentedBar } from "./SegmentedBar";

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
const num = (el: Element, a: string) => Number(el.getAttribute(a));

const segs = [
  { label: "worktrees", value: 16, paint: { series: 0 as const } },
  { label: "build", value: 9, paint: { series: 1 as const } },
  { label: "logs", value: 5, paint: { series: 2 as const } },
];

describe("SegmentedBar", () => {
  it("segments_lie_end_to_end_with_the_rest_outlined", () => {
    motionOff();
    const { container } = render(<SegmentedBar label="Storage" segments={segs} max={50} format={(n) => `${n} GB`} />);
    size(300);
    const r = Array.from(container.querySelectorAll(".chart-seg"));
    expect(r).toHaveLength(3);
    expect(num(r[0]!, "x")).toBe(0);
    expect(num(r[1]!, "x")).toBeCloseTo(num(r[0]!, "x") + num(r[0]!, "width"));
    expect(num(r[2]!, "x")).toBeCloseTo(num(r[1]!, "x") + num(r[1]!, "width"));
    expect(num(r[0]!, "width")).toBeCloseTo((16 / 50) * 300);
    expect(r[0]!.getAttribute("fill")).toBe("var(--series-0)");
    const rest = container.querySelector(".chart-outline")!;
    expect(num(rest, "x")).toBeCloseTo((30 / 50) * 300);
    expect(num(rest, "x") + num(rest, "width")).toBeCloseTo(300);
    expect(container.querySelectorAll(".chart-tick").length).toBeGreaterThan(1);
    expect(cells()).toEqual(["16 GB", "9 GB", "5 GB"]);
    noNaN(container);
  });

  it("a_total_over_max_clamps_the_drawing_but_reads_true", () => {
    motionOff();
    const { container } = render(<SegmentedBar label="S" segments={segs} max={20} />);
    size(200);
    const r = Array.from(container.querySelectorAll(".chart-seg"));
    expect(num(r[2]!, "x") + num(r[2]!, "width")).toBeLessThanOrEqual(200);
    expect(num(r[2]!, "width")).toBe(0);
    expect(container.querySelector(".chart-outline")).toBeNull();
    expect(cells()).toEqual(["16", "9", "5"]);
  });

  it("a_zero_total_draws_the_outline_and_no_data", () => {
    motionOff();
    const { container } = render(<SegmentedBar label="S" segments={segs.map((s) => ({ ...s, value: 0 }))} max={50} />);
    size(200);
    expect(container.querySelector(".chart-nodata")?.textContent).toBe("No data");
    expect(container.querySelectorAll(".chart-outline")).toHaveLength(1);
    expect(container.querySelectorAll(".chart-seg")).toHaveLength(0);
    noNaN(container);
  });

  it("segments_wipe_in_once", () => {
    vi.useFakeTimers();
    const { container } = render(<SegmentedBar label="S" segments={segs} max={50} />);
    size(300);
    const entering = () => Array.from(container.querySelectorAll(".chart-seg")).map((r) => r.hasAttribute("data-enter"));
    expect(entering()).toEqual([true, true, true]);
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(entering()).toEqual([false, false, false]);
  });

  it("hovering_a_segment_shows_its_tooltip", () => {
    motionOff();
    const { container } = render(<SegmentedBar label="S" segments={segs} max={50} format={(n) => `${n} GB`} />);
    size(300);
    fireEvent.mouseEnter(container.querySelectorAll(".chart-seg-hit")[1]!);
    expect(container.querySelector("[role=tooltip]")?.textContent).toBe("build9 GB");
    fireEvent.mouseLeave(container.querySelectorAll(".chart-seg-hit")[1]!);
    expect(container.querySelector("[role=tooltip]")).toBeNull();
  });

  it("the_legend_moves_to_the_table_when_compact", () => {
    motionOff();
    const { container } = render(<SegmentedBar label="S" segments={segs} max={50} />);
    size(layout.chartCompact + 20);
    expect(container.querySelectorAll(".chart-legend li")).toHaveLength(3);
    size(layout.chartCompact - 20);
    expect(container.querySelector(".chart-legend")).toBeNull();
    expect(cells()).toHaveLength(3);
  });

  it("draws_once_the_size_arrives_after_mounting_empty", () => {
    motionOff();
    const { container, rerender } = render(<SegmentedBar label="S" segments={[]} max={50} />);
    size(300);
    rerender(<SegmentedBar label="S" segments={segs} max={50} />);
    expect(container.querySelectorAll(".chart-seg")).toHaveLength(3);
  });
});

describe("Meter", () => {
  const marks = [{ at: 48, label: "warn 48", tone: "caution" as const }];

  it("the_meter_fills_to_its_value_and_marks_its_thresholds", () => {
    motionOff();
    const { container } = render(<Meter label="Budget" value={30} max={60} marks={marks} format={(n) => `$${n}`} />);
    size(300);
    const fill = container.querySelector(".chart-meter-fill")!;
    expect(fill.getAttribute("fill")).toBe("var(--series-0)");
    const bar = container.querySelector(".chart-outline")!;
    expect(num(fill, "width")).toBeCloseTo((num(bar, "width") * 30) / 60 - 2);
    const m = container.querySelector(".chart-meter-mark")!;
    expect(num(m, "x1")).toBeCloseTo((48 / 60) * 300);
    expect(m.getAttribute("stroke")).toBe("var(--tone-caution)");
    const label = container.querySelector(".chart-mark-label")!;
    expect(label.textContent).toBe("warn 48");
    expect(label.getAttribute("stroke")).toBeNull();
    expect(cells()).toEqual(["$30", "$48"]);
    expect(container.querySelector(".chart-legend")).toBeNull();
    noNaN(container);
  });

  it("a_meter_over_its_max_fills_to_the_end_and_reads_true", () => {
    motionOff();
    const { container } = render(<Meter label="B" value={90} max={60} format={(n) => `$${n}`} />);
    size(300);
    expect(num(container.querySelector(".chart-meter-fill")!, "width")).toBeCloseTo(298);
    expect(cells()).toEqual(["$90"]);
    fireEvent.mouseEnter(container.querySelector(".chart-seg-hit")!);
    expect(container.querySelector("[role=tooltip]")?.textContent).toBe("B$90");
  });

  it("hovering_a_threshold_shows_its_tooltip", () => {
    motionOff();
    const { container } = render(<Meter label="B" value={10} max={60} marks={marks} />);
    size(300);
    fireEvent.mouseEnter(container.querySelectorAll(".chart-seg-hit")[1]!);
    expect(container.querySelector("[role=tooltip]")?.textContent).toBe("warn 4848");
  });

  it("the_meter_fill_wipes_in_once", () => {
    vi.useFakeTimers();
    const { container } = render(<Meter label="B" value={10} max={60} />);
    size(300);
    expect(container.querySelector(".chart-meter-fill")!.hasAttribute("data-enter")).toBe(true);
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(container.querySelector(".chart-meter-fill")!.hasAttribute("data-enter")).toBe(false);
  });
});

describe("Donut", () => {
  const slices = [
    { label: "fleet", value: 50, paint: { series: 0 as const } },
    { label: "portal", value: 30, paint: { series: 1 as const } },
    { label: "other", value: 20, paint: { series: "other" as const } },
  ];

  it("donut_arcs_sum_to_the_whole_and_show_percentages_in_the_legend", () => {
    motionOff();
    const { container } = render(<Donut label="Cost" slices={slices} center="$100" format={(n) => `$${n}`} />);
    size(300);
    const arcs = Array.from(container.querySelectorAll(".chart-donut-arc"));
    expect(arcs).toHaveLength(3);
    expect(arcs.reduce((s, a) => s + num(a, "data-share"), 0)).toBeCloseTo(1);
    expect(Array.from(container.querySelectorAll(".chart-legend li")).map((l) => l.textContent)).toEqual(["fleet 50%", "portal 30%", "other 20%"]);
    expect(container.querySelector(".chart-donut-center")?.textContent).toBe("$100");
    expect(container.querySelectorAll(".chart-donut-ticks .chart-tick")).toHaveLength(4);
    expect(cells()).toEqual(["$50", "$30", "$20"]);
    noNaN(container);
  });

  it("arcs_start_at_twelve_and_close_without_a_gap", () => {
    motionOff();
    const { container } = render(<Donut label="D" slices={slices} />);
    size(300);
    const arcs = Array.from(container.querySelectorAll(".chart-donut-arc"));
    const pts = (a: Element) => a.getAttribute("d")!.match(/-?\d+(\.\d+)?/g)!.map(Number);
    const first = pts(arcs[0]!);
    const last = pts(arcs[2]!);
    expect(first[0]).toBeCloseTo(last[last.length - 2]!);
    expect(first[1]).toBeCloseTo(last[last.length - 1]!);
    expect(first[0]).toBeCloseTo(layout.chartPlot / 2 + 3);
  });

  it("a_single_full_slice_draws_a_finite_ring", () => {
    motionOff();
    const { container } = render(<Donut label="D" slices={[{ label: "all", value: 7, paint: { series: 0 } }]} />);
    size(300);
    expect(container.querySelectorAll("circle.chart-donut-arc")).toHaveLength(1);
    expect(container.querySelector(".chart-legend")).toBeNull();
    noNaN(container);
  });

  it("a_zero_total_draws_the_outline_and_no_data", () => {
    motionOff();
    const { container } = render(<Donut label="D" slices={slices.map((s) => ({ ...s, value: 0 }))} />);
    size(300);
    expect(container.querySelector(".chart-nodata")?.textContent).toBe("No data");
    expect(container.querySelectorAll(".chart-outline")).toHaveLength(1);
    expect(container.querySelectorAll(".chart-donut-arc")).toHaveLength(0);
    noNaN(container);
  });

  it("the_donut_sweeps_once", () => {
    vi.useFakeTimers();
    const { container } = render(<Donut label="D" slices={slices} />);
    size(300);
    const sweep = () => Array.from(container.querySelectorAll(".chart-donut-arc")).map((a) => a.getAttribute("data-enter"));
    expect(sweep()).toEqual(["sweep", "sweep", "sweep"]);
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(sweep()).toEqual([null, null, null]);
  });

  it("hovering_a_slice_shows_its_tooltip", () => {
    motionOff();
    const { container } = render(<Donut label="D" slices={slices} format={(n) => `$${n}`} />);
    size(300);
    fireEvent.mouseEnter(container.querySelectorAll(".chart-donut-arc")[1]!);
    expect(container.querySelector("[role=tooltip]")?.textContent).toBe("portal$30 · 30%");
  });

  it("the_legend_moves_to_the_table_when_compact", () => {
    motionOff();
    const { container } = render(<Donut label="D" slices={slices} />);
    size(layout.chartCompact - 20);
    expect(container.querySelector(".chart-legend")).toBeNull();
    expect(cells()).toHaveLength(3);
  });
});

describe("survive all zero", () => {
  it("segmented_bar_meter_and_donut_survive_all_zero", () => {
    motionOff();
    const zero = segs.map((s) => ({ ...s, value: 0 }));
    const a = render(<SegmentedBar label="S" segments={zero} max={0} />);
    const b = render(<Meter label="M" value={0} max={0} marks={[{ at: 0, label: "z", tone: "danger" }]} />);
    const c = render(<Donut label="D" slices={zero} />);
    const d = render(<SegmentedBar label="S2" segments={[{ ...segs[0]!, value: Number.NaN }, segs[1]!]} max={Number.NaN} />);
    const e = render(<Meter label="M2" value={Number.POSITIVE_INFINITY} max={Number.NaN} />);
    act(() => observed?.([{ contentRect: { width: 300, height: 100 } }]));
    for (const r of [a, b, c, d, e]) noNaN(r.container);
  });
});
