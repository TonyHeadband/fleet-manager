import type { Release } from "../api/generated/Release";
import type { Task } from "../api/generated/Task";
import { taskOutcome } from "../tasks/model";

// Ports of fleet-api's `parse_date`, `release_progress` and `today`, and of releases.rs's `format_day`, `local_day`
// and `ordered`.

const isLeap = (y: number) => (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
const daysInMonth = (y: number, m: number) => (m === 2 ? (isLeap(y) ? 29 : 28) : [4, 6, 9, 11].includes(m) ? 30 : 31);

/** Days since 1970-01-01 of a civil date (days-from-civil). */
function daysFromCivil(y: number, m: number, d: number): number {
  const yy = m <= 2 ? y - 1 : y;
  const era = Math.floor(yy / 400);
  const yoe = yy - era * 400;
  const doy = Math.floor((153 * (m > 2 ? m - 3 : m + 9) + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146_097 + doe - 719_468;
}

/** `"YYYY-MM-DD"` as days since 1970-01-01; null when it is not a real calendar date. */
export function parseDate(s: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (mo < 1 || mo > 12 || d < 1 || d > daysInMonth(y, mo)) return null;
  return daysFromCivil(y, mo, d);
}

/** `YYYY-MM-DD` of a day number counted from 1970-01-01 (civil-from-days). */
export function formatDay(days: number): string {
  const z = days + 719_468;
  const era = Math.floor(z / 146_097);
  const doe = z - era * 146_097;
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36_524) - Math.floor(doe / 146_096)) / 365);
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
  const mp = Math.floor((5 * doy + 2) / 153);
  const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
  const m = mp < 10 ? mp + 3 : mp - 9;
  const y = yoe + era * 400 + (m <= 2 ? 1 : 0);
  return `${String(y).padStart(4, "0")}-${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
}

/** The local calendar day of a Unix time, so "today" and a release day turn over at the person's midnight, not UTC's. */
export function localDay(at: number): number {
  const t = new Date(at * 1000);
  return daysFromCivil(t.getFullYear(), t.getMonth() + 1, t.getDate());
}

/** A release's two bars: tasks merged out of those that count, and days used out of the span, or overdue. */
export interface ReleaseProgress {
  merged: number;
  counted: number;
  daysUsed: number;
  daysTotal: number;
  overdueDays: number;
}

export function releaseProgress(r: Release, tasks: Task[], today: number): ReleaseProgress {
  let merged = 0;
  let counted = 0;
  for (const t of tasks.filter((t) => t.release === r.id && t.product === r.product)) {
    const o = taskOutcome(t);
    if (o === "dropped") continue;
    if (o === "merged" || o === "done_by_hand") merged += 1;
    counted += 1;
  }
  const start = parseDate(r.start) ?? today;
  const target = parseDate(r.target) ?? today;
  const total = Math.max(target - start + 1, 1);
  return {
    merged,
    counted,
    daysTotal: total,
    daysUsed: Math.min(Math.max(today - start + 1, 0), total),
    overdueDays: r.released_at === null ? Math.max(today - target, 0) : 0,
  };
}

/** Unreleased first, then released; newest start first within each, then by id. */
export function orderedReleases(rs: Release[]): Release[] {
  return [...rs].sort(
    (a, b) =>
      Number(a.released_at !== null) - Number(b.released_at !== null) ||
      (a.start < b.start ? 1 : a.start > b.start ? -1 : 0) ||
      b.id - a.id,
  );
}
