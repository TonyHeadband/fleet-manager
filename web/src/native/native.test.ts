import { afterEach, describe, expect, it, vi } from "vitest";
import { detectNative } from "./native";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async (cmd: string) => ({ start_daemon: { weStarted: false, url: "http://127.0.0.1:7878" }, version: "0.2.0", has_config: true })[cmd]) }));

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("the native bridge", () => {
  it("the_browser_has_no_native_bridge", async () => {
    expect(await detectNative()).toBeNull();
  });

  it("tauri_is_detected_by_its_internals", async () => {
    window.__TAURI_INTERNALS__ = {};
    const native = await detectNative();
    expect(native).not.toBeNull();
    expect(await native!.startDaemon()).toEqual({ weStarted: false, url: "http://127.0.0.1:7878" });
    expect(await native!.version()).toBe("0.2.0");
    expect(await native!.hasConfig()).toBe(true);
  });
});
