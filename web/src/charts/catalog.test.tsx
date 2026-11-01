import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CATALOG, ChartCatalog, SAMPLES } from "./index";
import type { Paint } from "./paint";

beforeEach(() => {
  document.documentElement.dataset.animations = "off";
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(public cb: (e: { contentRect: { width: number; height: number } }[]) => void) {
        queueMicrotask(() => act(() => this.cb([{ contentRect: { width: 400, height: 200 } }])));
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

const paints = (items: { paint?: Paint }[]) => items.map((i) => i.paint);

describe("ChartCatalog", () => {
  it("the_catalog_lists_all_eight_blocks_in_order", () => {
    expect(CATALOG.map((e) => e.name)).toEqual([
      "Stat",
      "Column bars",
      "Segmented bar",
      "Donut",
      "Time series",
      "Ranked bars",
      "Meter",
      "Status timeline",
    ]);
    expect(CATALOG.map((e) => `${e.minW}×${e.minH}`)).toEqual(["2×1", "2×2", "3×1", "2×2", "4×2", "2×2", "2×1", "4×2"]);
    render(<ChartCatalog />);
    for (const e of CATALOG) {
      const entry = screen.getByRole("region", { name: e.name });
      expect(within(entry).getByText(e.purpose)).toBeTruthy();
      expect(within(entry).getByText(`min ${e.minW}×${e.minH}`)).toBeTruthy();
    }
  });

  it("each_entry_previews_its_sample", () => {
    render(<ChartCatalog />);
    const labels = [
      SAMPLES.agentsLive.label,
      SAMPLES.agentsByStatus.label,
      SAMPLES.storageByKind.label,
      SAMPLES.costByProduct.label,
      SAMPLES.tokensPerHour.label,
      SAMPLES.topTasksByCost.label,
      SAMPLES.dailyBudget.label,
      SAMPLES.agentActivity.label,
    ];
    CATALOG.forEach((e, i) => {
      const entry = screen.getByRole("region", { name: e.name });
      expect(within(entry).getByRole("group", { name: labels[i] })).toBeTruthy();
      expect(entry.querySelector(".chart-hidden td")).not.toBeNull();
    });
  });

  it("samples_use_status_tones_only_for_status", () => {
    const status = [...paints(SAMPLES.agentsByStatus.bars), ...SAMPLES.agentActivity.lanes.flatMap((l) => l.bands.map((b) => ({ tone: b.tone })))];
    expect(status.every((p) => p && "tone" in p)).toBe(true);
    const categories = [
      ...paints(SAMPLES.storageByKind.segments),
      ...paints(SAMPLES.costByProduct.slices),
      ...paints(SAMPLES.tokensPerHour.series),
    ];
    expect(categories.every((p) => p && "series" in p)).toBe(true);
  });

  it("samples_are_deterministic_epoch_seconds", () => {
    const { from, to } = SAMPLES.tokensPerHour;
    expect(to).toBe(SAMPLES.agentActivity.to);
    expect(to - from).toBe(24 * 3600);
  });
});
