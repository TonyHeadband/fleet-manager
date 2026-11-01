import { act, render, renderHook, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paintVar } from "./paint";
import { linear, niceTicks, plainFormat } from "./scale";
import { useSize } from "./useSize";
import { useChartEntry } from "./useChartEntry";
import { ChartTooltip } from "./ChartTooltip";
import { Legend } from "./Legend";
import { ScaleTicks } from "./ScaleTicks";
import { HiddenTable } from "./HiddenTable";
import { NoData } from "./NoData";

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
  vi.unstubAllGlobals();
  delete document.documentElement.dataset.animations;
});

describe("chart shared pieces", () => {
  it("paint_maps_tones_series_and_other", () => {
    expect(paintVar({ tone: "done" })).toBe("var(--tone-done)");
    expect(paintVar({ series: 3 })).toBe("var(--series-3)");
    expect(paintVar({ series: "other" })).toBe("var(--tone-muted)");
  });

  it("nice_ticks_end_at_or_above_max", () => {
    expect(niceTicks(37)).toEqual([0, 10, 20, 30, 40]);
    expect(niceTicks(0)).toEqual([0]);
    expect(niceTicks(NaN)).toEqual([0]);
    const t = niceTicks(1234, 5);
    expect(t[0]).toBe(0);
    expect(t[t.length - 1]).toBeGreaterThanOrEqual(1234);
    expect(plainFormat(1200)).toBe((1200).toLocaleString());
    expect(plainFormat(1.25)).toBe((1.3).toLocaleString(undefined, { maximumFractionDigits: 1 }));
  });

  it("linear_maps_and_survives_a_flat_domain", () => {
    const f = linear(0, 10, 0, 100);
    expect(f(5)).toBe(50);
    expect(linear(3, 3, 7, 90)(3)).toBe(7);
    expect(linear(3, 3, 7, 90)(99)).toBe(7);
  });

  it("use_size_starts_at_zero_and_draws_nothing", () => {
    function Probe() {
      const [ref, { width }] = useSize<HTMLDivElement>();
      return (
        <div ref={ref} data-testid="box">
          {width > 0 && <svg data-testid="marks" width={width} />}
        </div>
      );
    }
    render(<Probe />);
    expect(screen.queryByTestId("marks")).toBeNull();
    act(() => observed?.([{ contentRect: { width: 0, height: 0 } }]));
    expect(screen.queryByTestId("marks")).toBeNull();
    act(() => observed?.([{ contentRect: { width: 240, height: 80 } }]));
    expect(screen.getByTestId("marks").getAttribute("width")).toBe("240");
  });

  it("the_entry_plays_once_across_updates", () => {
    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    const { result, rerender } = renderHook(({ d }) => useChartEntry(d), { initialProps: { d: false } });
    expect(result.current).toBe(false);
    rerender({ d: true });
    expect(result.current).toBe(true);
    rerender({ d: true });
    expect(result.current).toBe(false);
    rerender({ d: false });
    rerender({ d: true });
    expect(result.current).toBe(false);
  });

  it("the_entry_is_off_with_motion_off", () => {
    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    document.documentElement.dataset.animations = "off";
    const { result } = renderHook(() => useChartEntry(true));
    expect(result.current).toBe(false);
    document.documentElement.dataset.animations = "on";
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    const again = renderHook(() => useChartEntry(true));
    expect(again.result.current).toBe(false);
  });

  it("the_tooltip_shares_the_goal_tooltips_look", () => {
    render(<ChartTooltip x={10} y={20} lines={["Mon", "12 tasks"]} />);
    const tip = screen.getByRole("tooltip");
    expect(tip.className).toContain("fui-tooltip");
    expect(tip.textContent).toContain("12 tasks");
    const detail = readFileSync("src/shell/DetailPanel.tsx", "utf8");
    expect(detail).toMatch(/className="fui-tooltip goal-tooltip"/);
    const shell = readFileSync("src/shell/shell.css", "utf8");
    expect(shell).toMatch(/\.fui-tooltip \{[^}]*border-top-color: var\(--fill\)/);
    expect(shell).not.toMatch(/\.goal-tooltip \{[^}]*background/);
    const motion = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    expect(motion).toMatch(/\.fui-tooltip \{ animation: fui-tip /);
  });

  it("the_hidden_table_is_visually_hidden_but_read", () => {
    render(<HiddenTable caption="Tasks by day" rows={[["Mon", "12"], ["Tue", "3"]]} />);
    const table = screen.getByRole("table", { name: "Tasks by day" });
    expect(table.className).toContain("chart-hidden");
    expect(screen.getByRole("cell", { name: "12" })).toBeTruthy();
    expect(screen.getAllByRole("row")).toHaveLength(2);
    expect(readFileSync("src/charts/charts.css", "utf8")).toMatch(/\.chart-hidden \{[^}]*clip: rect\(0, 0, 0, 0\)/);
  });

  it("legend_scale_and_no_data_render", () => {
    render(
      <div>
        <Legend items={[{ label: "Open", paint: { series: 0 } }, { label: "Rest", paint: { series: "other" } }]} />
        <NoData />
        <svg>
          <ScaleTicks width={100} ticks={[0, 10, 20]} max={20} format={plainFormat} labels />
        </svg>
      </div>,
    );
    expect(screen.getByText("Open")).toBeTruthy();
    expect(screen.getByText("No data")).toBeTruthy();
    expect(screen.getByText("20")).toBeTruthy();
    expect(document.body.innerHTML).not.toContain("NaN");
  });

  it("scale_ticks_survive_a_zero_max_and_hide_labels", () => {
    render(
      <svg>
        <ScaleTicks width={100} ticks={[0]} max={0} format={plainFormat} labels={false} />
      </svg>,
    );
    expect(document.body.innerHTML).not.toContain("NaN");
    expect(document.querySelector("text")).toBeNull();
  });

  it("charts_css_uses_tokens_only", () => {
    const css = readFileSync("src/charts/charts.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/aspect-ratio/);
    expect(css).not.toMatch(/border-radius:\s*(?!0|var)/);
    expect(css).toMatch(/calc\(var\(--i\) \* var\(--motion-stagger\)\)/);
    expect(css).toMatch(/var\(--motion-wipe\)/);
    expect(css).toMatch(/prefers-reduced-motion: no-preference/);
    expect(css).toMatch(/data-animations="off"/);
  });
});
