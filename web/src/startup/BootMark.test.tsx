import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bootFrame } from "./bootTimeline";
import { bootBarWidth, BootMark } from "./BootMark";

afterEach(() => vi.restoreAllMocks());

describe("BootMark", () => {
  it("still_it_shows_the_finished_mark", () => {
    const { container } = render(<BootMark startedAt={performance.now()} animate={false} />);
    expect(container.querySelector(".boot-nose-mask")).toHaveAttribute("height", "19");
    expect(container.querySelector(".boot-tail-mask")).toHaveAttribute("height", "5");
    expect(container.querySelector(".boot-c-mask")).toHaveAttribute("stroke-dashoffset", "0");
    expect(container.querySelector(".boot-spark")).toHaveAttribute("opacity", "0");
    expect(container.querySelectorAll(".boot-ring .boot-dot")).toHaveLength(52);
    expect(container.querySelectorAll(".boot-ring .boot-marker")).toHaveLength(4);
  });

  it("still_it_starts_no_loop", () => {
    const raf = vi.spyOn(window, "requestAnimationFrame");
    render(<BootMark startedAt={performance.now()} animate={false} />);
    expect(raf).not.toHaveBeenCalled();
  });

  it("the_loop_stops_on_unmount", () => {
    let id = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => ++id);
    const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
    const { unmount } = render(<BootMark startedAt={performance.now()} animate />);
    unmount();
    expect(cancel).toHaveBeenLastCalledWith(id);
  });

  it("a_return_after_the_assembly_shows_the_finished_mark", () => {
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1);
    const { container } = render(<BootMark startedAt={performance.now() - 5000} animate />);
    expect(container.querySelector(".boot-nose-mask")).toHaveAttribute("height", "19");
    expect(container.querySelector(".boot-tail-mask")).toHaveAttribute("height", "5");
    expect(container.querySelector(".boot-c-mask")).toHaveAttribute("stroke-dashoffset", "0");
    expect(container.querySelector(".boot-spark")).toHaveAttribute("opacity", "0");
  });

  it("the_spark_core_fades_by_shrinking_only", () => {
    // Mid-fade, as the prototype: the core shrinks with the fade but stays fully opaque until it is gone.
    const ms = 2900;
    expect(bootFrame(ms).spark).toBeLessThan(1);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1);
    const { container } = render(<BootMark startedAt={performance.now() - ms} animate />);
    const core = container.querySelector(".boot-spark")!;
    expect(core).toHaveAttribute("opacity", "1");
    expect(Number(core.getAttribute("r"))).toBeLessThan(0.7);
  });

  it("the_trail_has_room_for_350ms_at_144hz", () => {
    const { container } = render(<BootMark startedAt={performance.now()} animate={false} />);
    expect(container.querySelectorAll(".boot-trail").length).toBeGreaterThanOrEqual(Math.ceil(0.35 * 144));
  });

  it("the_bar_matches_the_dot_ring", () => {
    expect(bootBarWidth()).toBeCloseTo(344.5);
  });
});
