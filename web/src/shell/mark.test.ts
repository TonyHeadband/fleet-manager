import { describe, expect, it } from "vitest";
import { depthAlpha, DOTS, onRing, project, ringAngles } from "./mark";

const flat = { x: 0, y: 0, spin: 0 };

describe("the mark's geometry", () => {
  it("there_are_56_dots_and_one_sits_on_each_cardinal", () => {
    expect(DOTS).toBe(56);
    const angles = ringAngles(DOTS);
    for (let k = 0; k < 4; k++) expect(angles[14 * k]).toBeCloseTo((Math.PI / 2) * k);
  });

  it("no_tilt_projects_flat_around_the_hub", () => {
    const p = project(onRing(10, 0), flat);
    expect(p.x).toBeCloseTo(32);
    expect(p.y).toBeCloseTo(24);
    expect(p.f).toBe(1);
  });

  it("spin_turns_clockwise", () => {
    const p = project(onRing(10, 0), { ...flat, spin: Math.PI / 2 });
    expect(p.x).toBeCloseTo(42);
    expect(p.y).toBeCloseTo(34);
  });

  it("the_far_side_dims_to_a_quarter", () => {
    expect(depthAlpha(10, 10)).toBe(0.25);
    expect(depthAlpha(-10, 10)).toBe(1);
    expect(depthAlpha(5, 0)).toBe(1);
  });

  it("a_tilted_point_moves_away_and_shrinks", () => {
    const p = project(onRing(10, 0), { ...flat, x: Math.PI / 2 });
    expect(Math.abs(p.z)).toBeCloseTo(10);
    expect(p.f).not.toBe(1);
  });
});
