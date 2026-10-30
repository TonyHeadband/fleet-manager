import { describe, expect, it, onTestFinished, vi } from "vitest";
import type { Release } from "../api/generated/Release";
import { fullTask } from "../test/fixtures";
import { formatDay, localDay, orderedReleases, parseDate, releaseProgress } from "./model";

// Ports of releases.rs's own tests and of fleet-api's release_progress, as egui's tests/releases.rs reads them.

const release = (id: number, version: string, start: string, target: string, released_at: number | null = null): Release => ({
  id,
  product: "p",
  version,
  start,
  target,
  released_at,
});
const day = (s: string) => parseDate(s)!;

describe("release model", () => {
  it("formats_days", () => {
    expect(formatDay(0)).toBe("1970-01-01");
    expect(formatDay(day("2028-02-29"))).toBe("2028-02-29");
    expect(formatDay(day("2026-10-26"))).toBe("2026-10-26");
  });

  it("parse_date_refuses_what_is_not_a_day", () => {
    for (const s of ["2026-02-30", "soon", "2026-1-01", "2027-02-29", ""]) expect(parseDate(s), s).toBeNull();
    expect(parseDate("1970-01-02")).toBe(1);
  });

  it("today_is_the_local_day", () => {
    // On a UTC runner the local day is UTC's and a UTC-only bug would pass: the clock is pinned well off UTC.
    vi.stubEnv("TZ", "Pacific/Auckland");
    onTestFinished(() => void vi.unstubAllEnvs());
    expect(new Date().getTimezoneOffset()).not.toBe(0);
    const at = day("2026-10-26") * 86_400 + 43_200;
    const d = new Date(at * 1000);
    const local = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    expect(formatDay(localDay(at))).toBe(local);
    // Just before local midnight it is still that day; just after, the next.
    const midnight = new Date(2026, 9, 27).getTime() / 1000;
    expect(formatDay(localDay(midnight - 1))).toBe("2026-10-26");
    expect(formatDay(localDay(midnight))).toBe("2026-10-27");
  });

  it("bars_match_release_progress", () => {
    const merged = fullTask("a", "task-a", "archived", "stopped", {
      release: 1,
      prs: [{ url: "u", repo: "core", state: "merged", merged_at: null }],
    });
    merged.repos[0]!.changed = true;
    const tasks = [
      merged,
      fullTask("b", "task-b", "archived", "stopped", { release: 1 }),
      fullTask("c", "task-c", "stopped", "stopped", { release: 1 }),
    ];
    expect(releaseProgress(release(1, "1.0", "2026-10-01", "2026-11-01"), tasks, day("2026-10-11"))).toEqual({
      merged: 1,
      counted: 2,
      daysUsed: 11,
      daysTotal: 32,
      overdueDays: 0,
    });
  });

  it("overdue_past_target", () => {
    expect(releaseProgress(release(1, "1.0", "2026-10-01", "2026-11-01"), [], day("2026-11-06")).overdueDays).toBe(5);
    expect(releaseProgress(release(1, "1.0", "2026-10-01", "2026-11-01", 1), [], day("2026-11-06")).overdueDays).toBe(0);
  });

  it("another_products_task_does_not_count", () => {
    const t = fullTask("c", "task-c", "stopped", "stopped", { release: 1, product: "q" });
    expect(releaseProgress(release(1, "1.0", "2026-10-01", "2026-11-01"), [t], day("2026-10-11")).counted).toBe(0);
  });

  it("ordered_unreleased_first_then_newest_start", () => {
    const rs = [
      release(1, "old", "2026-09-01", "2026-10-01"),
      release(2, "shipped", "2026-10-20", "2026-11-01", 5),
      release(3, "new", "2026-10-15", "2026-11-01"),
      release(4, "new too", "2026-10-15", "2026-11-01"),
    ];
    expect(orderedReleases(rs).map((r) => r.version)).toEqual(["new too", "new", "old", "shipped"]);
  });
});
