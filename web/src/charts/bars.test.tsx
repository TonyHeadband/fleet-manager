import { act, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { layout, motion, space } from "../generated/layout";
import { ColumnBars } from "./ColumnBars";
import { RankedBars } from "./RankedBars";

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
const heads = () => Array.from(document.querySelectorAll(".chart-hidden th")).map((c) => c.textContent);
const noNaN = (c: HTMLElement) => expect(c.innerHTML).not.toMatch(/NaN|Infinity/);

const bars = [
  { label: "Working", value: 4, paint: { tone: "active" as const } },
  { label: "Needs you", value: 2, paint: { tone: "attention" as const } },
  { label: "Queued", value: 0, paint: { tone: "muted" as const } },
];

describe("ColumnBars", () => {
  it("column_bars_draw_one_bar_per_category_with_its_value", () => {
    motionOff();
    const { container } = render(<ColumnBars label="Agents" bars={bars} format={(n) => `${n} up`} />);
    size(300);
    expect(screen.getByRole("group", { name: "Agents" })).toBeTruthy();
    expect(container.querySelectorAll(".chart-col-bar")).toHaveLength(3);
    expect(Array.from(container.querySelectorAll(".chart-col-value")).map((t) => t.textContent)).toEqual(["4 up", "2 up", "0 up"]);
    expect(Array.from(container.querySelectorAll(".chart-col-label")).map((t) => t.textContent)).toEqual(["Working", "Needs you", "Queued"]);
    const h = Array.from(container.querySelectorAll(".chart-col-bar")).map((r) => Number(r.getAttribute("height")));
    expect(h[0]!).toBeGreaterThan(h[1]!);
    expect(heads()).toEqual(["Working", "Needs you", "Queued"]);
    expect(cells()).toEqual(["4 up", "2 up", "0 up"]);
  });

  it("a_zero_column_is_a_one_pixel_sliver", () => {
    motionOff();
    const { container } = render(<ColumnBars label="A" bars={bars} />);
    size(300);
    const zero = container.querySelectorAll(".chart-col-bar")[2]!;
    expect(zero.getAttribute("height")).toBe("1");
  });

  it("bars_take_their_paint_not_their_rank", () => {
    motionOff();
    const fills = (bs: typeof bars) => {
      const { container, unmount } = render(<ColumnBars label="A" bars={bs} />);
      size(300);
      const m = new Map(
        Array.from(container.querySelectorAll(".chart-col-bar")).map((r, i) => [bs[i]!.label, r.getAttribute("fill")]),
      );
      unmount();
      return m;
    };
    const a = fills(bars);
    const b = fills([...bars].reverse());
    expect(a.get("Working")).toBe("var(--tone-active)");
    expect(b.get("Working")).toBe(a.get("Working"));
    expect(b.get("Queued")).toBe("var(--tone-muted)");
  });

  it("negative_values_draw_as_zero_but_read_true", () => {
    motionOff();
    const { container } = render(
      <ColumnBars label="A" bars={[{ label: "Net", value: -3, paint: { series: 0 } }, { label: "Up", value: 5, paint: { series: 1 } }]} />,
    );
    size(300);
    expect(container.querySelectorAll(".chart-col-bar")[0]!.getAttribute("height")).toBe("1");
    expect(cells()[0]).toBe("-3");
    fireEvent.mouseEnter(container.querySelectorAll(".chart-col-hit")[0]!);
    expect(screen.getByRole("tooltip").textContent).toContain("-3");
  });

  it("long_labels_truncate_but_read_whole", () => {
    motionOff();
    const long = "A very long product name indeed";
    const { container } = render(<ColumnBars label="A" bars={[{ label: long, value: 1, paint: { series: 0 } }]} />);
    size(100);
    const l = container.querySelector(".chart-col-label")!;
    expect(l.className).toContain("chart-col-label");
    const css = readFileSync("src/charts/charts.css", "utf8");
    expect(css).toMatch(/\.chart-col-label \{[^}]*-webkit-line-clamp: 2/);
    expect(heads()).toEqual([long]);
    fireEvent.mouseEnter(container.querySelector(".chart-col-hit")!);
    expect(screen.getByRole("tooltip").textContent).toContain(long);
  });

  it("hovering_a_bar_shows_its_tooltip", () => {
    motionOff();
    const { container } = render(<ColumnBars label="A" bars={bars} />);
    size(300);
    expect(screen.queryByRole("tooltip")).toBeNull();
    const hit = container.querySelectorAll(".chart-col-hit")[1]!;
    fireEvent.mouseEnter(hit);
    expect(screen.getByRole("tooltip").textContent).toBe("Needs you2");
    fireEvent.mouseLeave(hit);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("the_hover_target_is_the_whole_column", () => {
    motionOff();
    const { container } = render(<ColumnBars label="A" bars={bars} />);
    size(300);
    const hit = container.querySelectorAll(".chart-col-hit")[2]!;
    const bar = container.querySelectorAll(".chart-col-bar")[2]!;
    expect(Number(hit.getAttribute("height"))).toBeGreaterThan(Number(bar.getAttribute("height")));
    expect(Number(hit.getAttribute("width"))).toBeGreaterThan(Number(bar.getAttribute("width")));
  });

  it("value_labels_go_when_tiny", () => {
    motionOff();
    const { container } = render(<ColumnBars label="A" bars={bars} />);
    size(layout.chartTiny - 1);
    expect(container.querySelectorAll(".chart-col-bar")).toHaveLength(3);
    expect(container.querySelector(".chart-baseline")).toBeTruthy();
    expect(container.querySelectorAll(".chart-col-value")).toHaveLength(0);
    size(layout.chartTiny);
    expect(container.querySelectorAll(".chart-col-value")).toHaveLength(3);
  });

  it("column_bars_survive_all_zero", () => {
    motionOff();
    const { container } = render(
      <ColumnBars label="A" bars={bars.map((b) => ({ ...b, value: 0 }))} />,
    );
    size(300);
    expect(container.querySelectorAll(".chart-col-bar")).toHaveLength(3);
    noNaN(container);
  });

  it("column_bars_draw_nothing_at_width_zero_and_say_no_data_when_empty", () => {
    motionOff();
    const { container, rerender } = render(<ColumnBars label="A" bars={bars} />);
    expect(container.querySelectorAll(".chart-col-bar")).toHaveLength(0);
    noNaN(container);
    rerender(<ColumnBars label="A" bars={[]} />);
    expect(screen.getByText("No data")).toBeTruthy();
  });

  it("bars_enter_from_the_baseline_once", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<ColumnBars label="A" bars={bars} />);
    size(300);
    const entering = () => Array.from(container.querySelectorAll(".chart-col-bar")).map((r) => r.getAttribute("data-enter"));
    expect(entering()).toEqual(["grow", "grow", "grow"]);
    const style = (container.querySelectorAll(".chart-col-bar")[2] as SVGElement).style.getPropertyValue("--i");
    expect(style).toBe("2");
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(entering()).toEqual([null, null, null]);
    rerender(<ColumnBars label="A" bars={bars.map((b) => ({ ...b, value: b.value + 1 }))} />);
    expect(entering()).toEqual([null, null, null]);
  });

  it("the_stagger_is_capped", () => {
    vi.useFakeTimers();
    const many = Array.from({ length: motion.staggerCap + 5 }, (_, i) => ({ label: `L${i}`, value: i, paint: { series: 0 as const } }));
    const { container } = render(<ColumnBars label="A" bars={many} />);
    size(900);
    const last = container.querySelectorAll<SVGElement>(".chart-col-bar")[many.length - 1]!;
    expect(last.style.getPropertyValue("--i")).toBe(String(motion.staggerCap));
  });
});

const rows = [
  { label: "ghost-tasks", value: 9.8 },
  { label: "chat-pane", value: 7.6 },
  { label: "avatars", value: 2.15 },
];

describe("RankedBars", () => {
  it("ranked_bars_keep_the_callers_order", () => {
    motionOff();
    const shuffled = [rows[2]!, rows[0]!, rows[1]!];
    const { container } = render(<RankedBars label="Top tasks" rows={shuffled} format={(n) => `$${n}`} />);
    size(300);
    expect(screen.getByRole("group", { name: "Top tasks" })).toBeTruthy();
    expect(Array.from(container.querySelectorAll(".chart-rank-label")).map((t) => t.textContent)).toEqual(["avatars", "ghost-tasks", "chat-pane"]);
    expect(Array.from(container.querySelectorAll(".chart-rank-value")).map((t) => t.textContent)).toEqual(["$2.15", "$9.8", "$7.6"]);
    expect(heads()).toEqual(["avatars", "ghost-tasks", "chat-pane"]);
    expect(cells()).toEqual(["$2.15", "$9.8", "$7.6"]);
    const w = Array.from(container.querySelectorAll<HTMLElement>(".chart-rank-bar")).map((b) => parseFloat(b.style.width));
    expect(w[0]).toBeLessThan(w[2]!);
    expect(w[2]).toBeLessThan(w[1]!);
  });

  it("ranked_bars_default_to_series_zero_and_take_their_paint", () => {
    motionOff();
    const { container } = render(
      <RankedBars label="A" rows={[{ label: "a", value: 1 }, { label: "b", value: 2, paint: { series: 3 } }]} />,
    );
    size(300);
    const bg = Array.from(container.querySelectorAll<HTMLElement>(".chart-rank-bar")).map((b) => b.style.background);
    expect(bg).toEqual(["var(--series-0)", "var(--series-3)"]);
  });

  it("ranked_bars_draw_a_ticked_scale_under_the_rows", () => {
    motionOff();
    const { container } = render(<RankedBars label="A" rows={rows} />);
    size(300);
    expect(container.querySelector(".chart-scale")).toBeTruthy();
    expect(container.querySelectorAll(".chart-tick").length).toBeGreaterThan(1);
    expect(container.querySelectorAll(".chart-tick-label").length).toBeGreaterThan(1);
  });

  it("negative_values_draw_as_zero_but_read_true_in_ranked_bars", () => {
    motionOff();
    const { container } = render(<RankedBars label="A" rows={[{ label: "n", value: -4 }, { label: "p", value: 4 }]} />);
    size(300);
    expect(parseFloat(container.querySelectorAll<HTMLElement>(".chart-rank-bar")[0]!.style.width)).toBe(0);
    expect(cells()[0]).toBe("-4");
    fireEvent.mouseEnter(container.querySelectorAll(".chart-rank-row")[0]!);
    expect(screen.getByRole("tooltip").textContent).toContain("-4");
  });

  it("long_labels_truncate_but_read_whole_in_ranked_bars", () => {
    motionOff();
    const long = "refactor the whole scheduler end to end";
    const { container } = render(<RankedBars label="A" rows={[{ label: long, value: 1 }]} />);
    size(300);
    const css = readFileSync("src/charts/charts.css", "utf8");
    expect(css).toMatch(/\.chart-rank-label \{[^}]*text-overflow: ellipsis/);
    expect(heads()).toEqual([long]);
    fireEvent.mouseEnter(container.querySelector(".chart-rank-row")!);
    expect(screen.getByRole("tooltip").textContent).toContain(long);
  });

  it("hovering_a_row_shows_its_tooltip", () => {
    motionOff();
    const { container } = render(<RankedBars label="A" rows={rows} />);
    size(300);
    const row = container.querySelectorAll(".chart-rank-row")[1]!;
    fireEvent.mouseEnter(row);
    expect(screen.getByRole("tooltip").textContent).toBe("chat-pane7.6");
    fireEvent.mouseLeave(row);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("value_labels_go_when_tiny_in_ranked_bars", () => {
    motionOff();
    const { container } = render(<RankedBars label="A" rows={rows} />);
    const chrome = layout.chartLabelW + layout.chartValueW + 2 * space.buttonGap;
    size(chrome + layout.chartTiny - 1);
    expect(container.querySelectorAll(".chart-rank-bar")).toHaveLength(3);
    expect(container.querySelector(".chart-baseline")).toBeTruthy();
    expect(container.querySelectorAll(".chart-tick-label")).toHaveLength(0);
    size(chrome + layout.chartTiny);
    expect(container.querySelectorAll(".chart-tick-label").length).toBeGreaterThan(0);
  });

  it("ranked_bars_survive_all_zero", () => {
    motionOff();
    const { container } = render(<RankedBars label="A" rows={rows.map((r) => ({ ...r, value: 0 }))} />);
    size(300);
    expect(container.querySelectorAll(".chart-rank-bar")).toHaveLength(3);
    noNaN(container);
  });

  it("ranked_bars_draw_nothing_at_width_zero_and_say_no_data_when_empty", () => {
    motionOff();
    const { container, rerender } = render(<RankedBars label="A" rows={rows} />);
    expect(container.querySelectorAll(".chart-rank-bar")).toHaveLength(0);
    noNaN(container);
    rerender(<RankedBars label="A" rows={[]} />);
    expect(screen.getByText("No data")).toBeTruthy();
  });

  it("ranked_bars_wipe_in_once", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<RankedBars label="A" rows={rows} />);
    size(300);
    const entering = () => Array.from(container.querySelectorAll(".chart-rank-bar")).map((r) => r.getAttribute("data-enter"));
    expect(entering()).toEqual(["true", "true", "true"]);
    act(() => void vi.advanceTimersByTime(motion.wipe + motion.stagger * motion.staggerCap + 1));
    expect(entering()).toEqual([null, null, null]);
    rerender(<RankedBars label="A" rows={rows.map((r) => ({ ...r, value: r.value + 1 }))} />);
    expect(entering()).toEqual([null, null, null]);
  });
});

describe("late data and non-finite values", () => {
  it("column_bars_draw_when_data_arrives_after_mount", () => {
    motionOff();
    const { container, rerender } = render(<ColumnBars label="A" bars={[]} />);
    rerender(<ColumnBars label="A" bars={bars} />);
    size(300);
    expect(container.querySelectorAll(".chart-col-bar")).toHaveLength(3);
  });

  it("ranked_bars_draw_when_data_arrives_after_mount", () => {
    motionOff();
    const { container, rerender } = render(<RankedBars label="A" rows={[]} />);
    rerender(<RankedBars label="A" rows={rows} />);
    size(300);
    expect(container.querySelectorAll(".chart-rank-bar")).toHaveLength(3);
    expect(container.querySelector(".chart-scale")).toBeTruthy();
  });

  it("infinite_values_never_reach_an_attribute_or_style", () => {
    motionOff();
    const c = render(
      <ColumnBars label="A" bars={[{ label: "x", value: Infinity, paint: { series: 0 } }, { label: "y", value: 3, paint: { series: 1 } }]} />,
    );
    size(300);
    noNaN(c.container);
    c.unmount();
    const r = render(<RankedBars label="A" rows={[{ label: "x", value: Infinity }, { label: "y", value: 3 }]} />);
    size(300);
    noNaN(r.container);
  });
});
