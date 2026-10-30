import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// The app's permissions: an open_url call is refused unless a URL scope allows it.
describe("the app's capabilities", () => {
  it("links_to_the_web_may_be_opened", () => {
    const cap = JSON.parse(readFileSync("../crates/fleet-tauri/capabilities/default.json", "utf8")) as { permissions: string[] };
    expect(cap.permissions).toContain("opener:allow-default-urls");
  });
});
