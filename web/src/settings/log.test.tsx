import { readFileSync } from "node:fs";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LogDelta, LogEntry, LogFile, Native } from "../native/native";
import { fakeNative } from "../test/fixtures";
import { click, openApp, pending } from "../management/harness";
import { fileLinesBeforeLive, loggedSay, logRows, shown, ALL_LEVELS } from "./log";

// Ports of crates/fleet-gui/tests/log_view.rs and the unit tests in src/views/settings/log.rs.

const css = readFileSync("src/settings/settings.css", "utf8");

/** egui's `fleet_log::parse` of a log line, as the bridge hands it over (`line` is egui's `render`). */
function entry(text: string): LogEntry {
  const m = /^(\S+) (ERROR|WARN|INFO|DEBUG|TRACE) +(\S+) ([^\t]*)((?:\t[^\t]*)*)$/.exec(text);
  if (!m) return { time: 0, level: null, target: "", message: text, fields: [], line: text };
  const time = Date.parse(m[1]!) / 1000;
  const fields = m[5]!
    .split("\t")
    .slice(1)
    .map((f) => f.split("=") as [string, string]);
  return { time, level: m[2]!.toLowerCase() as LogEntry["level"], target: m[3]!, message: m[4]!, fields, line: text };
}
const file = (...lines: string[]): LogFile => ({ entries: lines.map(entry), notice: null });
/** A poll's answer: only what changed (#422). */
const delta = (appended: string[], over: Partial<LogDelta> = {}): LogDelta => ({
  reset: false,
  pop: false,
  appended: appended.map(entry),
  notice: null,
  ...over,
});

const PRE = "2026-10-24T10:00:01Z INFO  fleet_gui gui started";
const A = "2026-10-24T10:00:01Z INFO  fleet_gui a";
const B = "2026-10-24T10:00:02Z INFO  fleet_gui b";
const C = "2026-10-24T10:00:03Z INFO  fleet_gui c";
const APP = "2026-10-24T10:00:02Z INFO  fleet_gui app-line";
const DAEMON = ["2026-10-24T10:00:01Z INFO  fleet_daemon first-daemon", "2026-10-24T10:00:03Z INFO  fleet_daemon last-daemon"];

describe("The merge, as egui's", () => {
  const all = (app: LogFile, daemon: LogFile, live: LogEntry[] = []) => logRows(app.entries, live, daemon.entries, "both");

  it("both_merges_by_time", () => {
    const rows = all(file(APP), file(...DAEMON));
    expect(rows.map((r) => r.entry.message)).toEqual(["first-daemon", "app-line", "last-daemon"]);
    expect(logRows(file(APP).entries, [], file(...DAEMON).entries, "app").map((r) => r.entry.message)).toEqual(["app-line"]);
    expect(logRows(file(APP).entries, [], file(...DAEMON).entries, "daemon")).toHaveLength(2);
  });

  it("ties_list_the_app_line_first", () => {
    const rows = all(file("2026-10-24T10:00:01Z INFO  fleet_gui app-line"), file("2026-10-24T10:00:01Z INFO  fleet_daemon daemon-line"));
    expect(rows.map((r) => r.entry.message)).toEqual(["app-line", "daemon-line"]);
  });

  it("search_matches_fields", () => {
    const rows = all(
      file("2026-10-24T10:00:01Z INFO  fleet_gui spawned\tagent_id=7f3a", "2026-10-24T10:00:02Z INFO  fleet_gui other\tagent_id=0000"),
      file(),
    );
    const found = shown(rows, ALL_LEVELS, "7F3A");
    expect(found).toHaveLength(1);
    expect(found[0]!.entry.line).toContain("agent_id=7f3a");
  });

  it("debug_is_hidden_by_default", () => {
    const rows = all(file("2026-10-24T10:00:01Z DEBUG fleet_gui noisy", "2026-10-24T10:00:02Z INFO  fleet_gui quiet"), file());
    expect(shown(rows, ALL_LEVELS, "").map((r) => r.entry.message)).toEqual(["quiet"]);
    expect(shown(rows, { ...ALL_LEVELS, debug: true }, "")).toHaveLength(2);
  });

  it("level_toggle_hides_errors", () => {
    const rows = all(
      file(
        "2026-10-24T10:00:01Z ERROR fleet_gui broken",
        "2026-10-24T10:00:02Z WARN  fleet_gui careful",
        "2026-10-24T10:00:03Z INFO  fleet_gui fine",
      ),
      file(),
    );
    expect(shown(rows, { ...ALL_LEVELS, error: false }, "")).toHaveLength(2);
    expect(shown(rows, { ...ALL_LEVELS, error: false, warning: false }, "")).toHaveLength(1);
    expect(shown(rows, { error: false, warning: false, info: false, debug: false }, "")).toHaveLength(0);
  });

  it("unparsed_lines_ignore_level_toggles", () => {
    const rows = all(file(), {
      entries: [
        entry("2026-10-24T10:00:01Z INFO  fleet_daemon started"),
        { ...entry("thread 'main' panicked at src/main.rs:1"), time: Date.parse("2026-10-24T10:00:01Z") / 1000 },
      ],
      notice: null,
    });
    const left = shown(rows, { error: false, warning: false, info: false, debug: false }, "");
    expect(left.map((r) => r.entry.message)).toEqual(["thread 'main' panicked at src/main.rs:1"]);
  });

  it("a_file_line_from_the_same_second_as_the_first_live_one_is_listed", () => {
    expect(fileLinesBeforeLive([PRE, A, B].map(entry), [A, B].map(entry))).toBe(1);
  });

  it("file_lines_the_capped_live_log_dropped_are_listed", () => {
    expect(fileLinesBeforeLive([PRE, A, B, C].map(entry), [B, C].map(entry))).toBe(2);
  });

  it("a_file_that_shares_no_line_with_the_live_log_is_listed_whole", () => {
    expect(fileLinesBeforeLive([PRE, A].map(entry), [B, C].map(entry))).toBe(2);
    expect(fileLinesBeforeLive([PRE, A].map(entry), [])).toBe(2);
  });

  it("a_live_notice_the_file_filtered_out_does_not_hide_the_overlap", () => {
    const notice = "2026-10-24T10:00:01Z ERROR fleet_gui shown in the footer only";
    expect(fileLinesBeforeLive([PRE, A, B].map(entry), [notice, A, B].map(entry))).toBe(1);
  });

  it("an_app_line_in_the_file_and_the_live_log_is_listed_once", () => {
    const old = "2026-10-24T09:00:00Z INFO  fleet_gui old-run";
    const rows = logRows([old, A].map(entry), [A].map(entry), [], "both");
    expect(rows.map((r) => r.entry.message)).toEqual(["old-run", "a"]);
  });

  it("a_file_line_from_the_second_the_live_log_began_is_listed", () => {
    const rows = logRows([PRE, A].map(entry), [A].map(entry), [], "both");
    expect(rows.map((r) => r.entry.message)).toEqual(["gui started", "a"]);
  });
});

/** A bridge whose logs are these files and this live log; `poll` answers each `logPoll`. */
function logs(app: LogFile, daemon: LogFile, over: Partial<Native> = {}) {
  let handler: ((e: { seq: number; entry: LogEntry }) => void) | null = null;
  const native = fakeNative({
    logOpen: vi.fn().mockResolvedValue({ app, daemon }),
    onLogEntry: vi.fn(async (h) => {
      handler = h;
      return () => (handler = null);
    }),
    ...over,
  });
  const emit = (seq: number, text: string) => act(() => handler?.({ seq, entry: entry(text) }));
  return { native, emit };
}

const rowTexts = () => screen.queryAllByTestId("log-row").map((r) => r.textContent);

async function logTab(native: Native) {
  await openApp("Settings", "Log", { native });
}

beforeEach(() => localStorage.clear());
afterEach(() => vi.useRealTimers());

describe("Settings → Log", () => {
  it("missing_daemon_log_says_so", async () => {
    const { native } = logs(file(APP), { entries: [], notice: "No daemon log at /s/daemon.log" });
    await logTab(native);
    expect(await screen.findByText("No daemon log at /s/daemon.log")).toBeInTheDocument();
    expect(screen.getByText(/app-line/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "App" }));
    expect(screen.queryByText(/No daemon log at/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Daemon" }));
    expect(screen.getByText("No daemon log at /s/daemon.log")).toBeInTheDocument();
  });

  it("missing_app_log_says_so", async () => {
    const { native } = logs({ entries: [], notice: "No app log at /s/gui.log" }, file(...DAEMON));
    await logTab(native);
    expect(await screen.findByText("No app log at /s/gui.log")).toBeInTheDocument();
  });

  it("both_merges_by_time_in_the_view", async () => {
    const { native } = logs(file(APP), file(...DAEMON));
    await logTab(native);
    await waitFor(() => expect(rowTexts()).toHaveLength(3));
    expect(rowTexts()[0]).toMatch(/^10:00:01.*fleet_daemon first-daemon/);
  });

  it("new_app_entries_appear_live", async () => {
    const { native, emit } = logs(file(), file());
    await logTab(native);
    await waitFor(() => expect(native.logLive).toHaveBeenCalled());
    emit(1, "2026-10-24T10:00:05Z WARN  fleet_gui live-event");
    expect(rowTexts().at(-1)).toContain("live-event");
  });

  it("a_live_line_already_in_the_snapshot_is_not_listed_twice", async () => {
    const held = pending<{ entries: LogEntry[]; seq: number }>();
    const { native, emit } = logs(file(), file(), { logLive: vi.fn().mockReturnValue(held.promise) });
    await logTab(native);
    emit(1, B);
    emit(2, C);
    await act(async () => held.resolve({ entries: [entry(B)], seq: 1 }));
    expect(rowTexts().map((t) => t?.slice(-1))).toEqual(["b", "c"]);
  });

  it("the_daemon_file_is_followed", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { native } = logs(file(), file(DAEMON[0]!), {
      logPoll: vi
        .fn()
        .mockResolvedValueOnce(delta([DAEMON[1]!]))
        .mockResolvedValue(null),
    });
    await logTab(native);
    await act(() => vi.advanceTimersByTimeAsync(1100));
    expect(rowTexts()).toHaveLength(2);
  });

  it("a_held_back_daemon_line_that_comes_back_whole_replaces_its_fragment", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fragment = "2026-10-24T10:00:03Z INFO  fleet_daemon hal";
    const whole = "2026-10-24T10:00:03Z INFO  fleet_daemon half done";
    const { native } = logs(file(), file(DAEMON[0]!, fragment), {
      logPoll: vi
        .fn()
        .mockResolvedValueOnce(delta([whole], { pop: true }))
        .mockResolvedValue(null),
    });
    await logTab(native);
    await waitFor(() => expect(rowTexts()).toHaveLength(2));
    await act(() => vi.advanceTimersByTimeAsync(1100));
    expect(rowTexts()).toHaveLength(2);
    expect(rowTexts()[1]).toContain("half done");
  });

  it("a_reset_daemon_file_replaces_its_rows", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const rotated = "2026-10-24T10:00:09Z INFO  fleet_daemon rotated";
    const { native } = logs(file(), file(...DAEMON), {
      logPoll: vi
        .fn()
        .mockResolvedValueOnce(delta([rotated], { reset: true }))
        .mockResolvedValue(null),
    });
    await logTab(native);
    await act(() => vi.advanceTimersByTimeAsync(1100));
    expect(rowTexts()).toHaveLength(1);
    expect(rowTexts()[0]).toContain("rotated");
  });

  it("a_hidden_log_tab_reads_no_file", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { native } = logs(file(), file());
    await openApp("Settings", null, { native });
    await act(() => vi.advanceTimersByTimeAsync(3000));
    expect(native.logOpen).not.toHaveBeenCalled();
    expect(native.logPoll).not.toHaveBeenCalled();
    await click("Log");
    await act(() => vi.advanceTimersByTimeAsync(3100));
    expect(native.logOpen).toHaveBeenCalledTimes(1);
    expect(vi.mocked(native.logPoll).mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(vi.mocked(native.logPoll).mock.calls.length).toBeLessThanOrEqual(4);
    await click("App");
    const polls = vi.mocked(native.logPoll).mock.calls.length;
    await act(() => vi.advanceTimersByTimeAsync(3000));
    expect(native.logPoll).toHaveBeenCalledTimes(polls);
  });

  it("a_slow_poll_is_not_asked_again_while_in_flight", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const held = pending<LogFile | null>();
    const { native } = logs(file(), file(), { logPoll: vi.fn().mockReturnValue(held.promise) });
    await logTab(native);
    await act(() => vi.advanceTimersByTimeAsync(5000));
    expect(native.logPoll).toHaveBeenCalledTimes(1);
    await act(async () => held.resolve(null));
  });

  it("each_row_leads_with_the_levels_dot", async () => {
    const { native } = logs(
      file(
        "2026-10-24T10:00:01Z ERROR fleet_gui e",
        "2026-10-24T10:00:02Z WARN  fleet_gui w",
        "2026-10-24T10:00:03Z INFO  fleet_gui i",
        "2026-10-24T10:00:04Z DEBUG fleet_gui d",
      ),
      { entries: [{ ...entry("stray"), time: Date.parse("2026-10-24T10:00:05Z") / 1000 }], notice: null },
    );
    await logTab(native);
    await userEvent.click(screen.getByRole("button", { name: "debug" }));
    await waitFor(() => expect(rowTexts()).toHaveLength(5));
    const dots = screen.getAllByTestId("log-row").map((r) => r.querySelector(".log-dot")?.getAttribute("data-level") ?? null);
    expect(dots).toEqual(["error", "warn", "info", "debug", null]);
  });

  it("a_cut_off_row_shows_the_whole_line_on_hover", async () => {
    const long = `2026-10-24T10:00:01Z INFO  fleet_gui ${"x".repeat(400)}\tagent_id=7f3a`;
    const { native } = logs(file(long), file());
    await logTab(native);
    await waitFor(() => expect(rowTexts()).toHaveLength(1));
    expect(screen.getByTestId("log-row").querySelector("[title]")).toHaveAttribute("title", expect.stringContaining("\tagent_id=7f3a"));
  });

  it("rows_render_a_window_and_stick_to_the_bottom", async () => {
    const lines = Array.from({ length: 300 }, (_, n) => `2026-10-24T10:00:01Z INFO  fleet_gui row-${n}`);
    const { native } = logs(file(...lines), file());
    await logTab(native);
    expect(await screen.findByText(/row-299$/)).toBeInTheDocument();
    expect(screen.queryByText(/row-0$/)).toBeNull();
  });

  it("in_a_browser_the_log_says_only_in_the_app", async () => {
    await openApp("Settings", "Log", {});
    expect(screen.getByText("Only in the app")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Both" })).toBeInTheDocument();
  });

  it("the_log_css_uses_tokens_only", () => {
    const rules = css.slice(css.indexOf("/* egui's log.rs"));
    expect(rules.length).toBeLessThan(css.length);
    for (const literal of rules.match(/\b\d+(\.\d+)?(px|rem|em)\b/g) ?? []) expect(literal).toBe("1px");
    for (const token of ["--settings-log-search", "--settings-log-dot", "--settings-log-row"]) expect(rules).toContain(`var(${token})`);
  });
});

describe("Open log and the notices", () => {
  it("open_log_opens_settings_log", async () => {
    const { native } = logs(file(), file(), { logFileError: vi.fn().mockResolvedValue("permission denied") });
    await openApp("Kanban", null, { native });
    // The file error is the first notice, so History has a line to open.
    expect(await within(screen.getByRole("contentinfo")).findByText("Logging to file is off: permission denied")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    await userEvent.click(screen.getByRole("button", { name: "Open log →" }));
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Log");
  });

  it("a_notice_is_logged_through_the_bridge", async () => {
    const { native } = logs(file(), file(), { logFileError: vi.fn().mockResolvedValue("permission denied") });
    await openApp("Kanban", null, { native });
    await waitFor(() => expect(native.logNotice).toHaveBeenCalledWith("warning", "Logging to file is off: permission denied", undefined));
  });
});

describe("The final review's fixes", () => {
  it("a_notice_about_an_agent_is_logged_with_its_agent", () => {
    const native = fakeNative();
    const show = vi.fn();
    loggedSay(native, show)("warning", "Could not grant", "a1");
    expect(show).toHaveBeenCalledWith("warning", "Could not grant", "a1");
    expect(native.logNotice).toHaveBeenCalledWith("warning", "Could not grant", "a1");
  });

  it("the_live_snapshot_waits_until_the_window_listens", async () => {
    const listening = pending<() => void>();
    const { native } = logs(file(), file(), { onLogEntry: vi.fn().mockReturnValue(listening.promise) });
    await logTab(native);
    expect(native.logLive).not.toHaveBeenCalled();
    await act(async () => listening.resolve(() => {}));
    expect(native.logLive).toHaveBeenCalledTimes(1);
  });

  it("a_poll_answered_after_the_tab_was_left_still_counts", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const held = pending<LogDelta | null>();
    const { native } = logs(file(), file(), { logPoll: vi.fn().mockReturnValueOnce(held.promise).mockResolvedValue(null) });
    await logTab(native);
    await act(() => vi.advanceTimersByTimeAsync(1100));
    expect(native.logPoll).toHaveBeenCalledTimes(1);
    await click("App");
    await act(async () => held.resolve(delta(DAEMON)));
    await click("Log");
    await waitFor(() => expect(rowTexts()).toHaveLength(2));
  });

  it("the_level_toggles_are_buttons_and_the_sources_chips", async () => {
    const { native } = logs(file(), file());
    await logTab(native);
    expect(screen.getByRole("button", { name: "Both" })).toHaveClass("chip");
    for (const l of ["error", "warning", "info", "debug"]) expect(screen.getByRole("button", { name: l })).not.toHaveClass("chip");
    expect(screen.getByRole("button", { name: "debug" })).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByRole("button", { name: "info" })).toHaveAttribute("aria-pressed", "true");
  });
});
