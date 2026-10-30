import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { detectNative } from "./native";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(
    async (cmd: string) => ({ start_daemon: { weStarted: false, url: "http://127.0.0.1:7878" }, version: "0.2.0", has_config: true })[cmd],
  ),
}));

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

describe("choices", () => {
  it("each_write_is_numbered_for_its_page_load", async () => {
    window.__TAURI_INTERNALS__ = {};
    const native = (await detectNative())!;
    await native.writeChoice("fleet.theme", "dark");
    await native.writeChoice("fleet.theme", "light");
    const calls = vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === "write_choice")
      .map(([, args]) => args as Record<string, unknown>);
    // The bridge drops a write that arrives after a newer one for its key (#416).
    expect(calls.map((a) => [a.key, a.value])).toEqual([
      ["fleet.theme", "dark"],
      ["fleet.theme", "light"],
    ]);
    expect(calls[1]!.seq as number).toBeGreaterThan(calls[0]!.seq as number);
    expect(typeof calls[0]!.session).toBe("string");
    expect(calls[1]!.session).toBe(calls[0]!.session);
  });
});

describe("links in the app", () => {
  it("a_new_tab_link_goes_to_the_browser_in_the_app", async () => {
    const { fakeNative } = await import("../test/fixtures");
    const { installAppNative } = await import("./native");
    const native = fakeNative();
    installAppNative(native);
    const a = document.createElement("a");
    a.href = "https://example.invalid/pr/1";
    a.target = "_blank";
    a.textContent = "PR";
    document.body.append(a);
    a.click();
    expect(native.openUrl).toHaveBeenCalledWith("https://example.invalid/pr/1");
    a.remove();
  });
});
