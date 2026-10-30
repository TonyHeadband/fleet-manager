import { describe, expect, it } from "vitest";
import fixture from "../generated/login.json";
import { claudeLoginScript, ghLoginScript } from "./login";

// The scripts fleet-client builds (crates/fleet-client/tests/login.rs writes the fixture), built again here for a
// browser to copy.
describe("login scripts, as fleet-client's", () => {
  it.each(fixture.cases.map((c, i) => [i, c] as const))("case %i matches", (_, c) => {
    expect(claudeLoginScript(c.persona, c.claude, c.tasks_root)).toBe(c.claude_script);
    expect(ghLoginScript(c.persona, c.gh)).toBe(c.gh_script);
  });
});
