import { readFileSync } from "node:fs";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { DiffReport } from "../api/generated/DiffReport";
import { DiffView } from "./DiffView";

const file = (path: string, body: string[]) =>
  [`diff --git a/${path} b/${path}`, `--- a/${path}`, `+++ b/${path}`, ...body].join("\n");
const store = file("crates/fleet-daemon/src/store.rs", ["@@ -1,3 +1,3 @@", " keep", "-old line", "+new line", " tail"]);
const added = ["diff --git a/crates/fleet-daemon/tests/backfill.rs b/crates/fleet-daemon/tests/backfill.rs", "--- /dev/null", "+++ b/crates/fleet-daemon/tests/backfill.rs", "@@ -0,0 +1,2 @@", "+fn a() {}", "+fn b() {}"].join("\n");
const gone = ["diff --git a/plans/old.md b/plans/old.md", "--- a/plans/old.md", "+++ /dev/null", "@@ -1,1 +0,0 @@", "-x"].join("\n");

const loaded = (workspaces: DiffReport["workspaces"]) => ({ state: "loaded" as const, report: { workspaces } });
const two = loaded([
  { repo: "fleet-manager", diff: `${store}\n${added}`, truncated: false },
  { repo: "fleet-manager-internal", diff: gone, truncated: false },
]);

const tree = () => screen.getByRole("navigation", { name: "Changed files" });
const viewer = () => screen.getByRole("region", { name: "File" });

describe("DiffView", () => {
  it("lists_each_repos_files_under_their_folders", () => {
    render(<DiffView diff={two} />);
    const t = within(tree());
    expect(t.getByText("fleet-manager")).toBeInTheDocument();
    expect(t.getByText("fleet-manager-internal")).toBeInTheDocument();
    expect(t.getByText("crates/fleet-daemon/src/")).toBeInTheDocument();
    const row = t.getByRole("button", { name: "crates/fleet-daemon/tests/backfill.rs" });
    expect(row).toHaveTextContent("A");
    expect(row).toHaveTextContent("backfill.rs");
    expect(row).toHaveTextContent("+2");
    expect(within(t.getByText("fleet-manager").closest("[data-repo]") as HTMLElement).getByText("+3")).toBeInTheDocument();
    expect(t.getByRole("button", { name: "plans/old.md" })).toHaveTextContent("D");
  });

  it("picks_the_first_changed_file", () => {
    render(<DiffView diff={two} />);
    expect(within(tree()).getByRole("button", { name: "crates/fleet-daemon/src/store.rs" })).toHaveAttribute("aria-pressed", "true");
    expect(within(viewer()).getByText("crates/fleet-daemon/src/store.rs")).toBeInTheDocument();
  });

  it("shows_no_changes_for_an_unchanged_repo", () => {
    render(<DiffView diff={loaded([{ repo: "empty", diff: "", truncated: false }, { repo: "other", diff: gone, truncated: false }])} />);
    expect(within(tree()).getByText("No changes")).toBeInTheDocument();
    expect(within(tree()).getByRole("button", { name: "plans/old.md" })).toHaveAttribute("aria-pressed", "true");
  });

  it("is_split_by_default_and_switches_to_unified", async () => {
    render(<DiffView diff={two} />);
    expect(screen.getByRole("button", { name: "Split" })).toHaveAttribute("aria-pressed", "true");
    const rows = () => [...viewer().querySelectorAll("[data-row]")].map((r) => r.textContent);
    expect(viewer().querySelector('[data-mode="split"]')).not.toBeNull();
    expect(rows().some((t) => t?.includes("old line") && t.includes("new line"))).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Unified" }));
    expect(screen.getByRole("button", { name: "Unified" })).toHaveAttribute("aria-pressed", "true");
    const lines = rows();
    expect(lines.findIndex((t) => t?.includes("old line"))).toBeLessThan(lines.findIndex((t) => t?.includes("new line")));
    expect(lines.some((t) => t?.includes("old line") && t.includes("new line"))).toBe(false);
  });

  it("shows_the_picked_file_on_press", async () => {
    render(<DiffView diff={two} />);
    await userEvent.click(within(tree()).getByRole("button", { name: "plans/old.md" }));
    expect(within(viewer()).getByText("plans/old.md")).toBeInTheDocument();
  });

  it("renders_only_the_picked_file", () => {
    const many = Array.from({ length: 50 }, (_, i) => file(`f${i}.rs`, ["@@ -1 +1 @@", `-old ${i}`, `+new ${i}`])).join("\n");
    render(<DiffView diff={loaded([{ repo: "r", diff: many, truncated: false }])} />);
    expect(viewer().querySelectorAll("[data-row]")).toHaveLength(1);
    expect(viewer()).toHaveTextContent("new 0");
    expect(viewer()).not.toHaveTextContent("new 1");
  });

  it("shows_the_cut_note_on_cut_files", async () => {
    render(<DiffView diff={loaded([{ repo: "r", diff: `${store}\n${added}`, truncated: true }])} />);
    expect(viewer()).not.toHaveTextContent("The diff was cut at the size limit.");
    await userEvent.click(within(tree()).getByRole("button", { name: "crates/fleet-daemon/tests/backfill.rs" }));
    expect(viewer()).toHaveTextContent("The diff was cut at the size limit.");
  });

  it("shows_a_binary_file_as_such", () => {
    const bin = ["diff --git a/img.png b/img.png", "Binary files a/img.png and b/img.png differ"].join("\n");
    render(<DiffView diff={loaded([{ repo: "r", diff: bin, truncated: false }])} />);
    expect(viewer()).toHaveTextContent("Binary file");
  });

  it("keeps_loading_and_failure_states", () => {
    const { rerender } = render(<DiffView diff={{ state: "loading" }} />);
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    rerender(<DiffView diff={{ state: "failed", message: "no such agent" }} />);
    expect(screen.getByRole("alert")).toHaveTextContent("no such agent");
  });

  it("diff_css_uses_tokens_only", () => {
    const css = readFileSync("src/diff/diff.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/aspect-ratio/);
    expect(css).toMatch(/var\(--layout-diff-tree\)/);
  });

  it("file_and_folder_names_keep_their_case_and_order", () => {
    const css = readFileSync("src/diff/diff.css", "utf8");
    // A right-to-left folder row moves its trailing slash to the front; the list row's caps would upcase the names.
    expect(css).not.toMatch(/direction: rtl/);
    expect(css).toMatch(/\.diff-tree \.list-row \{[^}]*text-transform: none;/);
  });
});
