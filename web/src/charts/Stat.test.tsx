import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Sparkline } from "./Sparkline";
import { Stat } from "./Stat";

let observed: ((entries: { contentRect: { width: number; height: number } }[]) => void) | null = null;

beforeEach(() => {
  observed = null;
  vi.useFakeTimers();
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

const size = (width: number, height = 56) => act(() => observed?.([{ contentRect: { width, height } }]));
const finishEntry = () => act(() => void vi.advanceTimersByTime(5000));

describe("Stat", () => {
  it("stat_shows_the_formatted_value", () => {
    document.documentElement.dataset.animations = "off";
    render(<Stat label="Agents live" value={1234} format={(n) => `${n} agents`} />);
    expect(screen.getByRole("group", { name: "Agents live" })).toBeTruthy();
    expect(screen.getAllByText("1234 agents").length).toBeGreaterThan(0);
  });

  it("the_delta_shows_its_arrow_and_tone", () => {
    document.documentElement.dataset.animations = "off";
    const { rerender, container } = render(
      <Stat label="L" value={11} delta={{ value: 3, period: "1h", tone: "good" }} />,
    );
    const delta = () => container.querySelector<HTMLElement>(".chart-stat-delta")!;
    expect(delta().textContent).toBe("▲ 3 / 1h");
    expect(delta().style.color).toBe("var(--tone-done)");
    rerender(<Stat label="L" value={11} delta={{ value: -2, period: "1h", tone: "bad" }} />);
    expect(delta().textContent).toBe("▼ 2 / 1h");
    expect(delta().style.color).toBe("var(--tone-danger)");
    rerender(<Stat label="L" value={11} delta={{ value: 0, period: "1h", tone: "neutral" }} />);
    expect(delta().style.color).toBe("var(--text-low)");
  });

  it("the_delta_and_period_are_in_the_table", () => {
    document.documentElement.dataset.animations = "off";
    render(<Stat label="L" value={11} delta={{ value: -2, period: "1h", tone: "bad" }} />);
    const cells = Array.from(document.querySelectorAll(".chart-hidden td")).map((c) => c.textContent);
    expect(cells).toEqual(["11", "-2 / 1h"]);
  });

  it("the_number_steps_up_on_entry_and_not_after", () => {
    const { container, rerender } = render(<Stat label="L" value={100} />);
    const shown = () => container.querySelector(".chart-stat-value")!.textContent;
    expect(shown()).toBe("0");
    act(() => void vi.advanceTimersByTime(100));
    const mid = Number(shown());
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(100);
    finishEntry();
    expect(shown()).toBe("100");
    rerender(<Stat label="L" value={250} />);
    expect(shown()).toBe("250");
    act(() => void vi.advanceTimersByTime(500));
    expect(shown()).toBe("250");
  });

  it("a_value_change_after_the_step_but_inside_the_entry_window_shows_at_once", () => {
    const { container, rerender } = render(<Stat label="L" value={100} />);
    const shown = () => container.querySelector(".chart-stat-value")!.textContent;
    act(() => void vi.advanceTimersByTime(500));
    expect(shown()).toBe("100");
    rerender(<Stat label="L" value={250} />);
    expect(shown()).toBe("250");
  });

  it("a_one_point_spark_draws_a_marker", () => {
    document.documentElement.dataset.animations = "off";
    const { container } = render(<Sparkline values={[4]} />);
    expect(container.querySelector(".chart-marker")).toBeNull();
    size(200);
    expect(container.querySelector(".chart-marker")).not.toBeNull();
    expect(container.querySelector("polyline")).toBeNull();
  });

  it("a_spark_with_several_points_draws_a_line", () => {
    document.documentElement.dataset.animations = "off";
    const { container } = render(<Sparkline values={[1, 3, 2]} />);
    size(200);
    const line = container.querySelector("polyline")!;
    expect(line.getAttribute("stroke")).toBe("var(--series-0)");
    expect(line.getAttribute("points")).not.toMatch(/NaN/);
  });

  it("stat_survives_all_zero", () => {
    document.documentElement.dataset.animations = "off";
    const { container } = render(<Stat label="L" value={0} delta={{ value: 0, period: "1h", tone: "neutral" }} spark={[0, 0, 0]} />);
    size(200);
    expect(container.innerHTML).not.toMatch(/NaN|Infinity/);
    expect(container.querySelector("polyline")).not.toBeNull();
  });
});
