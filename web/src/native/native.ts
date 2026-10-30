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
  /** Runs a tool's install command in its own terminal; resolves when that window closes. */
  installTool(command: string): Promise<void>;
  /** Takes the daemon's PATH, so terminals this app opens find what the daemon found. */
  adoptPath(path: string): Promise<void>;
  copyText(text: string): Promise<void>;
  openUrl(url: string): Promise<void>;
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
