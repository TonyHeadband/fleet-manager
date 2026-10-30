import type { Effort } from "../api/generated/Effort";
import type { PermissionMode } from "../api/generated/PermissionMode";
import type { ToolId } from "../api/generated/ToolId";

/**
 * What the web UI asks of the machine it runs on, when it runs in the Tauri app. In a browser there is none
 * (`null`), and each native action shows disabled with its reason, as Open in terminal does.
 */
export interface Native {
  /** The app's own version, to compare with the daemon's. */
  version(): Promise<string>;
  /** False on a first run: no config file yet. */
  hasConfig(): Promise<boolean>;
  /**
   * Finds the daemon or starts it, and answers with its address (known only once it is up, on port 0); rejects with
   * the reason egui's failure screen shows.
   */
  startDaemon(): Promise<{ weStarted: boolean; url: string; local: boolean }>;
  /** Where the daemon keeps its state, for the failure screen. */
  stateDir(): Promise<string>;
  /** The first run's fields as egui fills them, `~` for the home folder. */
  firstRunDefaults(): Promise<{ root: string; personas: string }>;
  /** Creates the tasks folder and writes the first config; rejects with egui's text for what is wrong. */
  writeInitialConfig(root: string, personas: string): Promise<void>;
  /** The OS's folder dialog, opened at `start` (else where the last pick was made, else home); null when cancelled. */
  pickFolder(start: string | null): Promise<string | null>;
  /** Runs the daemon's install command for `tool` in its own terminal; resolves when that window closes. */
  installTool(tool: ToolId): Promise<void>;
  /** Opens a persona's login in a terminal; `script` is set when no terminal was found, to run by hand. */
  runLogin(persona: string, which: "claude" | "gh"): Promise<{ script: string | null }>;
  /** Hands a token to the persona's gh; rejects with gh's message, the token masked. */
  savePat(persona: string, token: string): Promise<void>;
  /** The wizard's "Add an app menu shortcut". */
  addMenuShortcut(): Promise<void>;
  /** Settings' Shortcuts: this app's own entries, never egui's. */
  shortcuts(): Promise<ShortcutsReply>;
  /** Add or Remove the ticked places, or Update the existing ones to this exe; rejects with the OS's reason. */
  changeShortcuts(action: "add" | "remove" | "update", menu: boolean, desktop: boolean): Promise<ShortcutsReply>;
  /** Takes the daemon's PATH, asked of the daemon itself, so terminals this app opens find what the daemon found. */
  adoptPath(): Promise<void>;
  copyText(text: string): Promise<void>;
  openUrl(url: string): Promise<void>;
  /** The window's close flow: what the Rust side, which caught the close, asks the page to show. */
  onClose(handler: (e: CloseEvent) => void): () => void;
  answerClose(answer: CloseAnswer, remember: boolean): Promise<void>;
  /** The board is up (or gone): before it, a close just closes. */
  setCloseArmed(armed: boolean): Promise<void>;
  /** Open in terminal: the agent's attach command, looked up by the app from the daemon. */
  openTerminal(agent: string): Promise<void>;
  /** Settings' daemon settings as `config.toml` has them. */
  readDaemonSettings(): Promise<DaemonSettings>;
  /** Writes `change` to `config.toml`, then restarts the daemon; a failed write rejects without restarting. Rejects
   * with egui's text. */
  saveAndRestart(change: SettingsChange): Promise<RestartReply>;
  /** Writes `change` to `config.toml` without restarting: the relaunched wizard's folders. Rejects with egui's text. */
  saveSettings(change: SettingsChange): Promise<void>;
  /** Settings → Log: both files' last 1 MiB, read once on the tab's first show. */
  logOpen(): Promise<{ app: LogFile; daemon: LogFile }>;
  /** What `daemon.log` changed since; null when nothing a row or notice shows changed. */
  logPoll(): Promise<LogDelta | null>;
  /** What this window logged since it started, the last 5000 lines, and the `seq` of the last one. */
  logLive(): Promise<{ entries: LogEntry[]; seq: number }>;
  /** Each line this window logs from now on, with its place in the live log. Resolves, with the way to stop, once
   * the window listens: a snapshot taken after that misses nothing. */
  onLogEntry(handler: (e: { seq: number; entry: LogEntry }) => void): Promise<() => void>;
  /** The page's notice as a log line, as egui's notices are. */
  logNotice(kind: "error" | "warning" | "success" | "info", text: string, agentId?: string): Promise<void>;
  /** Why `gui.log` could not be opened, when it could not. */
  logFileError(): Promise<string | null>;
  /** The Daemon tab's rows, the pidfile read afresh. */
  daemonInfo(): Promise<DaemonInfo>;
  /** What the release check says; the notice leaves out a skipped release, Settings still shows it. */
  checkUpdate(): Promise<UpdateCheck>;
  skipVersion(version: string): Promise<void>;
  /** The board's choices and Settings' preferences in egui's `gui.toml`, shared with it. */
  readChoices(): Promise<SavedChoices>;
  /** Resolves once `gui.toml` is written; rejects with why not. */
  writeChoice(key: string, value: string | null): Promise<void>;
}

export type CloseEvent = { phase: "asking" } | { phase: "stopping" } | { phase: "stop-failed"; message: string };
export type CloseAnswer = "keep" | "stop" | "cancel" | "close-anyway";
export interface Release {
  version: string;
  notes: string;
  page: string;
}

/** `unknown` until a check answers. */
export type UpdateCheck =
  { kind: "unknown" } | { kind: "current" } | { kind: "failed" } | ({ kind: "available"; skipped: boolean } & Release);

export interface ShortcutsReply {
  state: "none" | "current" | "stale";
  menu: boolean;
  desktop: boolean;
}

/** `gui.toml`'s values under its own names (`dark`, `space_grotesk`, `keep_running`). */
export interface SavedChoices {
  persona: string | null;
  product: string | null;
  display: string;
  theme: string;
  uiFont: string;
  monoFont: string;
  animations: boolean;
  stickyAlerts: boolean;
  closeChoice: string | null;
}

let current: Native | null = null;

/** The bridge this page runs with, for code that has no `native` prop (New task's Copy). */
export const appNative = () => current;

/**
 * Sets the page's bridge. In the app, a link that opens a new tab goes to the browser instead: the webview has no
 * tabs, and would otherwise do nothing.
 */
export function installAppNative(native: Native | null) {
  current = native;
  if (!native) return;
  document.addEventListener("click", (e) => {
    const link = (e.target as Element | null)?.closest?.('a[target="_blank"]') as HTMLAnchorElement | null;
    if (!link || e.defaultPrevented) return;
    e.preventDefault();
    void native.openUrl(link.href);
  });
}

/** Copies through the app when there is one, else through the browser. */
export const copyText = (native: Native | null, text: string) =>
  native ? native.copyText(text) : (navigator.clipboard?.writeText(text) ?? Promise.resolve());

/** Tauri's webview defines these before the page runs. */
declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

/** The Tauri bridge inside the app, `null` in a browser (and in tests, which pass their own fake). */
export async function detectNative(): Promise<Native | null> {
  if (typeof window === "undefined" || window.__TAURI_INTERNALS__ === undefined) return null;
  const { tauriNative } = await import("./tauri");
  return tauriNative();
}

/** What `gui.toml` reads as with nothing saved, or unreadable, as in egui. */
export const DEFAULT_CHOICES: SavedChoices = {
  persona: null,
  product: null,
  display: "compact",
  theme: "system",
  uiFont: "plex",
  monoFont: "plex",
  animations: true,
  stickyAlerts: true,
  closeChoice: null,
};

export interface LimitsDto {
  maxBreakdownVersions: number;
  maxSteps: number;
  maxParallelSpecialists: number;
  maxPrAttempts: number;
  /** null is no cap. */
  maxConcurrentTasks: number | null;
  maxActiveProducts: number | null;
}

export interface PrDto {
  style: string;
  attribution: string;
  draft: boolean;
  labels: string[];
  gateBuild: "shared" | "clean";
}

export interface DaemonSettings {
  /** `~`-shortened. */
  tasksRoot: string;
  personasDir: string;
  limits: LimitsDto;
  pr: PrDto;
  defaultMode: PermissionMode;
  defaultEffort: Effort;
  /** What `~` stands for. */
  home: string;
  /** The folders the daemon runs with, `~`-shortened; `tasksRoot` and `personasDir` are the config's, which may wait
   * for a restart. */
  runningTasksRoot: string;
  runningPersonasDir: string;
}

/** One of Settings' Saves, or Restart daemon with nothing to write (egui's `RestartFor`). */
export type SettingsChange =
  | { kind: "tasksRoot"; text: string }
  | { kind: "personasDir"; text: string }
  | ({ kind: "limits" } & LimitsDto)
  | ({ kind: "pr" } & PrDto)
  | { kind: "agentDefaults"; mode: PermissionMode; effort: Effort }
  | { kind: "restart" };

export interface RestartReply {
  /** `alreadyRunning`: no `daemon.pid` to stop, so the old daemon still runs. */
  outcome: "started" | "alreadyRunning";
  url: string;
  local: boolean;
  weStarted: boolean;
}

export interface DaemonInfo {
  address: string;
  startedBy: "this window" | "already running";
  stateDir: string;
  configFile: string;
  pid: string | null;
}

/** One log line: what a row draws, and the whole rendered line Search and the hover use. */
export interface LogEntry {
  /** Unix seconds, UTC. */
  time: number;
  /** null for a line not in the log format (a panic, stderr noise). */
  level: "error" | "warn" | "info" | "debug" | "trace" | null;
  target: string;
  message: string;
  fields: [string, string][];
  line: string;
}

/** A log file's lines and why it shows none, when it does not. */
export interface LogFile {
  entries: LogEntry[];
  notice: string | null;
}

/** A poll's answer, spliced into the file the page has (`crates/fleet-tauri/src/log.rs`, `LogDelta`): after a reset
 * the lines start over; otherwise a held-back last line is dropped (`pop`) before the new ones. */
export interface LogDelta {
  reset: boolean;
  pop: boolean;
  appended: LogEntry[];
  notice: string | null;
}
