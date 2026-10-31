import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { RadarMark } from "./RadarMark";

describe("RadarMark", () => {
  it("the_mark_has_no_sweep_and_56_dots", () => {
    const { container } = render(<RadarMark />);
    expect(container.querySelectorAll(".radar-ring circle")).toHaveLength(57);
    expect(container.querySelector(".radar-ring polygon")).toBeNull();
    expect(container.querySelectorAll("svg > g:not(.radar-ring) > :is(polygon, path)")).toHaveLength(3);
  });

  it("a_spinning_mark_turns_its_ring_every_15s", () => {
    const { container } = render(<RadarMark spin />);
    const ring = container.querySelector<SVGGElement>(".radar-ring")!;
    expect(ring).toHaveAttribute("data-spin");
    expect(ring.style.animationDuration).toBe("15000ms");
  });

  it("the_mark_says_how_far_its_ring_spills_below_the_square", () => {
    const { container } = render(<RadarMark />);
    const spill = container.querySelector("svg")!.style.getPropertyValue("--mark-spill-bottom");
    expect(Number(spill)).toBeCloseTo((34 + 26.5 * 1.3 + 0.35 - 64) / 64);
  });
});
