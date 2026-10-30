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
  startDaemon(): Promise<{ weStarted: boolean; url: string }>;
  /** Where the daemon keeps its state, for the failure screen. */
  stateDir(): Promise<string>;
}

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
