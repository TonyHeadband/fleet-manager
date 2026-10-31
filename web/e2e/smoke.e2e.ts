import { $, expect } from "@wdio/globals";

// The app, launched against the throwaway world scripts/tauri-smoke.sh made: a daemon up with a ready persona. The
// boot view's steps are Vitest's (src/startup); here the whole path is real, from the webview to the daemon.
describe("Fleet Manager on Tauri", () => {
  it("opens on the board against a real daemon", async () => {
    await expect($("h1=Filters")).toBeDisplayed({ wait: 30_000 });
    // A ready persona: no setup wizard over the board.
    await expect($("h2*=Your first persona")).not.toBeDisplayed();
    await expect($("aria/Working")).toBeDisplayed();
  });
});
