import { describe, expect, it } from "vitest";
import type { CheckoutInfo } from "../api/generated/CheckoutInfo";
import type { Product } from "../api/generated/Product";
import { persona, product, repoInfo } from "../test/fixtures";
import {
  checkoutLabel, emptyDraft, fleetBlocker, keptRelease, openReleases, personaBlocker, picks, setPersona, shortPath, startBlocker, startOf, toolErrorText, wantsLocal, workflowLine, type Draft, type Listings,
} from "./model";

// Ported from crates/fleet-gui/src/components/new_task.rs and tests/app.rs; the names are Lite's.
const draft = (over: Partial<Draft> = {}): Draft => ({ ...emptyDraft(), persona: "me", ...over });
const checkout = (path: string, main: boolean, branch: string | null = "main"): CheckoutInfo => ({ path, main, branch, commit: "abc1234", subject: "s" });
const repos = [
  repoInfo("both", { path: "/src/both", url: "https://example.invalid/both.git" }),
  repoInfo("web", { path: null, url: "https://example.invalid/web.git" }),
  repoInfo("tool", { path: "/src/tool", url: null }),
];

describe("New task's rules, as Lite's SpawnState", () => {
  it("start_choice_offered_only_with_path_and_url", () => {
    expect(wantsLocal(repos, draft(), "both"), "a repo with a URL starts from it by default").toBe(false);
    expect(wantsLocal(repos, draft({ starts: { both: "/src/both" } }), "both")).toBe(true);
    expect(wantsLocal(repos, draft({ starts: { both: null } }), "both")).toBe(false);
    expect(wantsLocal(repos, draft(), "tool"), "a path alone is the only way").toBe(true);
    expect(wantsLocal(repos, draft(), "web")).toBe(false);
  });

  it("a_local_start_sends_the_main_checkout_for_the_repos_own_path", () => {
    const listings: Listings = { "/src/both": { state: "ready", checkouts: [checkout("/real/both", true), checkout("/wt/feat", false, "feat")] } };
    expect(startOf(repos, draft({ starts: { both: "/src/both" } }), listings, "both")).toBe("/real/both");
    expect(startOf(repos, draft({ starts: { both: "/wt/feat" } }), listings, "both")).toBe("/wt/feat");
    expect(startOf(repos, draft(), listings, "both")).toBeNull();
    expect(picks(repos, draft({ chosen: ["both", "web"], starts: { both: "/wt/feat" } }), listings)).toEqual([
      { repo: "both", local: "/wt/feat" },
      { repo: "web", local: null },
    ]);
  });

  it("start_waits_for_a_local_start_to_list_and_stops_on_a_failure", () => {
    const d = draft({ chosen: ["web", "tool"] });
    expect(startBlocker(repos, d, {})).toBe("Loading the checkouts of tool…");
    expect(startBlocker(repos, d, { "/src/tool": { state: "loading" } })).toBe("Loading the checkouts of tool…");
    expect(startBlocker(repos, d, { "/src/tool": { state: "failed", error: "not a git repository" } })).toBe("Cannot list the checkouts of tool: not a git repository");
    expect(startBlocker(repos, d, { "/src/tool": { state: "ready", checkouts: [] } })).toBe("tool has no checkout to start from");
    expect(startBlocker(repos, d, { "/src/tool": { state: "ready", checkouts: [checkout("/src/tool", true)] } })).toBeNull();
  });

  it("the_workflow_line_reads_as_lites", () => {
    expect(workflowLine("core", { state: "found", base: "main", steps: ["fmt", "test"] })).toEqual({ text: "Checks on main: fmt, test", blocks: false });
    expect(workflowLine("core", { state: "missing", base: "main" })).toEqual({
      text: "core has no .fleet/workflow.yml on main. It lists the checks the gate runs before a pull request, so orchestrated tasks need it.",
      blocks: true,
    });
    expect(workflowLine("core", { state: "invalid", base: "main", error: "bad yaml" })).toEqual({ text: "core: .fleet/workflow.yml on main is invalid: bad yaml", blocks: true });
    expect(workflowLine("core", { state: "unreadable", base: "dev", error: "denied" })).toEqual({ text: "core: cannot read .fleet/workflow.yml on dev: denied", blocks: true });
  });

  it("tool_refusals_read_as_lites", () => {
    const fix = { install: null, link: "https://x.invalid", manager: null };
    expect(toolErrorText({ kind: "not-installed", tool: "gh", fix })).toBe("GitHub CLI (gh) is not installed");
    expect(toolErrorText({ kind: "too-old", tool: "git", found: "2.20", needed: "2.38", fix })).toBe("Git 2.20 is too old: 2.38 or newer is needed");
    expect(toolErrorText({ kind: "not-logged-in", tool: "claude", persona: null })).toBe("Claude Code is not logged in");
    expect(toolErrorText({ kind: "not-logged-in", tool: "claude", persona: "work" })).toBe("Claude Code is not logged in for work");
    expect(toolErrorText({ kind: "broken", tool: "git-bash", detail: "exit 1", fix })).toBe("Git Bash is installed but does not work: exit 1");
  });

  it("checkout_labels_shorten_long_paths", () => {
    expect(shortPath("/a/b")).toBe("/a/b");
    expect(shortPath("/home/me/src/core")).toBe("…/src/core");
    expect(shortPath("C:\\src\\core"), "drive, root and two parts").toBe("…/src/core");
    expect(shortPath("C:\\core")).toBe("C:\\core");
    expect(checkoutLabel(checkout("/home/me/src/core", true))).toBe("main  …/src/core");
    expect(checkoutLabel(checkout("/w/x", false, null))).toBe("detached  /w/x");
  });

  it("a_release_not_in_the_product_is_dropped", () => {
    const rel = (id: number, released: number | null) => ({ id, product: "p", version: `1.${id}`, start: "", target: "", released_at: released });
    const p: Product = product("p", "p", { releases: [rel(7, null), rel(8, 1)] });
    expect(openReleases(p).map((r) => r.id)).toEqual([7]);
    expect(keptRelease(p, 7)).toBe(7);
    expect(keptRelease(p, 8), "released since").toBeNull();
    expect(keptRelease(undefined, 7), "no product").toBeNull();
  });

  it("persona_and_fleet_blockers", () => {
    expect(personaBlocker([persona("me")], "me")).toBeNull();
    expect(personaBlocker([persona("me"), persona("draft", false)], "draft")).toBe("Pick a persona");
    expect(personaBlocker([persona("draft", false)], "draft")).toBe("Set up a persona first");
    expect(fleetBlocker(draft()), "every task is orchestrated, so every task needs its repo").toBe("Pick a repo");
    expect(fleetBlocker(draft({ chosen: ["a"] }))).toBeNull();
  });

  it("a_new_persona_drops_the_picks", () => {
    const d = setPersona(draft({ chosen: ["a"], starts: { a: null } }), "other");
    expect(d).toMatchObject({ persona: "other", chosen: [], starts: {} });
    expect(setPersona(d, "other")).toBe(d);
  });
});
