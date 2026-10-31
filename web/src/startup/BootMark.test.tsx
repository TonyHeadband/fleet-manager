import { render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bootBarWidth, BootMark } from "./BootMark";

afterEach(() => vi.restoreAllMocks());

describe("BootMark", () => {
  it("still_it_shows_the_finished_mark", () => {
    const { container } = render(<BootMark startedAt={performance.now()} animate={false} />);
    expect(container.querySelector(".boot-nose-mask")).toHaveAttribute("height", "19");
    expect(container.querySelector(".boot-tail-mask")).toHaveAttribute("height", "5");
    expect(container.querySelector(".boot-c-mask")).toHaveAttribute("stroke-dashoffset", "0");
    expect(container.querySelector(".boot-spark")).toHaveAttribute("opacity", "0");
    expect(container.querySelectorAll(".boot-ring .boot-dot")).toHaveLength(56);
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

  it("the_bar_matches_the_dot_ring", () => {
    expect(bootBarWidth()).toBeCloseTo(344.5);
  });
});
