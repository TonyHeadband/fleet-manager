import { describe, expect, it } from "vitest";
import { bootFrame } from "./bootTimeline";

describe("the boot timeline", () => {
  it("it_starts_from_a_point_tilted_with_nothing_shown", () => {
    const f = bootFrame(0);
    expect(f.scale).toBe(0);
    expect(f.tilt.x).toBeCloseTo(0.9 * Math.PI);
    expect(f.tilt.y).toBeCloseTo(0.6 * Math.PI);
    expect(f.tilt.spin).toBe(0);
    expect(f.meridians).toBe(1);
    expect([f.nose, f.tail, f.c, f.spark]).toEqual([0, 0, 0, 0]);
  });

  it("the_craft_waits_for_the_ring", () => {
    expect(bootFrame(400).nose).toBe(0);
    expect(bootFrame(450).nose).toBeGreaterThan(0);
  });

  it("the_ring_lands_face_on_at_1600", () => {
    const f = bootFrame(1600);
    expect(f.scale).toBe(1);
    expect(f.tilt.x).toBe(0);
    expect(f.tilt.y).toBe(0);
    expect(bootFrame(1700).meridians).toBe(0);
  });

  it("the_spin_is_one_constant_rate", () => {
    const step = (2 * Math.PI * 500) / 15000;
    expect(bootFrame(1000).tilt.spin - bootFrame(500).tilt.spin).toBeCloseTo(step);
    expect(bootFrame(20000).tilt.spin - bootFrame(19500).tilt.spin).toBeCloseTo(step);
  });

  it("nose_then_tail_then_c", () => {
    expect(bootFrame(1100).nose).toBe(1);
    expect(bootFrame(1100).tail).toBe(0);
    expect(bootFrame(1700).tail).toBe(1);
    expect(bootFrame(1700).c).toBe(0);
    expect(bootFrame(2700).c).toBe(1);
    expect(bootFrame(2700).spark).toBe(1);
  });

  it("the_spark_fades_out_by_3050", () => {
    const fading = bootFrame(2900).spark;
    expect(fading).toBeGreaterThan(0);
    expect(fading).toBeLessThan(1);
    expect(bootFrame(3050).spark).toBe(0);
  });
});
