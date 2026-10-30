import { useEffect, useRef, useState } from "react";
import type { LogEntry, LogFile, Native } from "../native/native";
import type { Notice } from "../shell/messages";

// A port of crates/fleet-gui/src/views/settings/log.rs's LogView: the files as the bridge reads them, this window's
// live lines, and egui's `rebuild` (merge, source, levels, search).

export type Source = "app" | "daemon" | "both";
export interface Levels {
  error: boolean;
  warning: boolean;
  info: boolean;
  debug: boolean;
}
/** egui's start: everything but debug. */
export const ALL_LEVELS: Levels = { error: true, warning: true, info: true, debug: false };

/** egui's `LOG_CAP`. */
const LOG_CAP = 5000;
/** egui's `POLL`: the daemon's file is checked at most this often while the tab shows. */
const POLL_MS = 1000;

/** egui's notices are tracing events: `show` puts it in the footer, and the bridge logs it, in the Log and gui.log. */
export function loggedSay(native: Native | null, show: (notice: Notice, text: string, agentId?: string) => void) {
  return (notice: Notice, text: string, agentId?: string) => {
    show(notice, text, agentId);
    void native?.logNotice(notice, text, agentId);
  };
}

export interface Row {
  entry: LogEntry;
  from: "app" | "daemon";
}

/**
 * egui's `file_lines_before_live`: how many of `gui.log`'s lines to list before the live ones, those up to the first
 * that the live log also holds. By identity (time and rendered line), not by time: a line logged in the same second
 * just before the window subscribed is in the file only, and so are lines the capped live log dropped.
 */
export function fileLinesBeforeLive(file: readonly LogEntry[], live: readonly LogEntry[]): number {
  if (live.length === 0) return file.length;
  const inLive = (e: LogEntry) => live.some((l) => l.time === e.time && l.line === e.line);
  const oldest = live[0]!.time;
  const at = file.findIndex((e) => e.time >= oldest && inLive(e));
  return at === -1 ? file.length : at;
}

/** The rows `source` shows, merged by time; on a tie the app's line comes first (egui `rebuild`). */
export function logRows(app: readonly LogEntry[], live: readonly LogEntry[], daemon: readonly LogEntry[], source: Source): Row[] {
  const appRows: Row[] =
    source === "daemon" ? [] : [...app.slice(0, fileLinesBeforeLive(app, live)), ...live].map((entry) => ({ entry, from: "app" }));
  const daemonRows: Row[] = source === "app" ? [] : daemon.map((entry) => ({ entry, from: "daemon" }));
  const out: Row[] = [];
  let a = 0;
  let d = 0;
  while (a < appRows.length || d < daemonRows.length) {
    const takeApp = d >= daemonRows.length || (a < appRows.length && appRows[a]!.entry.time <= daemonRows[d]!.entry.time);
    out.push(takeApp ? appRows[a++]! : daemonRows[d++]!);
  }
  return out;
}

const LEVEL: Record<NonNullable<LogEntry["level"]>, keyof Levels> = {
  error: "error",
  warn: "warning",
  info: "info",
  debug: "debug",
  trace: "debug",
};

/** A line with no level always passes; Search is a case-insensitive part of the rendered line, fields included. */
export function shown(rows: readonly Row[], levels: Levels, search: string): Row[] {
  const needle = search.toLowerCase();
  return rows.filter(
    ({ entry }) => (entry.level === null || levels[LEVEL[entry.level]]) && (needle === "" || entry.line.toLowerCase().includes(needle)),
  );
}

const capped = (l: LogEntry[]) => (l.length > LOG_CAP ? l.slice(l.length - LOG_CAP) : l);

/**
 * The files and live lines while `showing`: both files read once on the first show, the daemon's polled every second
 * (never twice at once), the live log taken afresh on each show and followed while it lasts. Hidden, nothing is read.
 */
export function useLog(native: Native | null, showing: boolean) {
  const [app, setApp] = useState<LogFile | null>(null);
  const [daemon, setDaemon] = useState<LogFile | null>(null);
  const [live, setLive] = useState<LogEntry[]>([]);
  const opened = useRef(false);
  // The first read landed: polls follow it, never overtake it (egui's `loaded`).
  const loaded = useRef(false);

  useEffect(() => {
    if (!native || !showing) return;
    let on = true;
    if (!opened.current) {
      opened.current = true;
      native.logOpen().then(
        // Kept even when the tab was left meanwhile: it is read once, so the next show needs it.
        (f) => {
          setApp(f.app);
          setDaemon(f.daemon);
          loaded.current = true;
        },
        () => (opened.current = false),
      );
    }
    // Lines that arrive before the snapshot wait for it; those it already has (by `seq`) are dropped. The snapshot is
    // taken once the window listens, so no line falls between the two.
    let seq: number | null = null;
    const early: { seq: number; entry: LogEntry }[] = [];
    let stop = () => {};
    native
      .onLogEntry((e) => {
        if (seq === null) early.push(e);
        else if (e.seq > seq) {
          seq = e.seq;
          setLive((l) => capped([...l, e.entry]));
        }
      })
      .then((unlisten) => {
        if (!on) return unlisten();
        stop = unlisten;
        return native.logLive().then((snap) => {
          if (!on) return;
          const after = early.filter((e) => e.seq > snap.seq);
          seq = after.at(-1)?.seq ?? snap.seq;
          setLive(capped([...snap.entries, ...after.map((e) => e.entry)]));
        });
      })
      .catch(() => {});
    let polling = false;
    const timer = setInterval(() => {
      if (polling || !loaded.current) return;
      polling = true;
      native
        .logPoll()
        // Kept even when the tab was left meanwhile: the bridge has moved past these lines.
        .then(
          (f) => f && setDaemon(f),
          () => {},
        )
        .finally(() => (polling = false));
    }, POLL_MS);
    return () => {
      on = false;
      stop();
      clearInterval(timer);
    };
  }, [native, showing]);

  return { app, daemon, live };
}
