import { render, screen, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { agent, schema, workspace } from "../test/fixtures";
import { Details, ReposRow } from "./Details";
import { workspaceTip } from "./workspaceTip";

// Ported from crates/fleet-gui/tests/panels.rs; the names are Lite's.
const NOW = 1300;
const detailsAgent = (over: Partial<Agent> = {}) =>
  agent({ status_since: 1000, workspaces: [{ ...workspace("core"), branch: "fleet/idea" }, { ...workspace("portal"), branch: "fleet/idea" }], has_changes: true, paused: true, ...over });
const show = (a: Agent, s: UiSchema = schema) => render(<Details schema={s} agent={a} now={NOW} />);

describe("the panel's details", () => {
  it("the_details_show_the_status_every_badge_and_the_branch", () => {
    show(detailsAgent({ status: "needs_you", blocked_reason: "permission" }));
    for (const text of ["Needs you", "5m", "Permission needed", "Has changes", "Paused", "Branch", "fleet/idea"]) expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByText("core"), "the repos have their own row").toBeNull();
  });

  it("a_stopped_agents_details_show_its_cause_and_size", () => {
    show(detailsAgent({ status: "stopped", stop_cause: "crashed", disk_bytes: 7_600_000_000 }));
    for (const text of ["Stopped", "Crashed", "7.6 GB"]) expect(screen.getByText(text)).toBeInTheDocument();
  });

  it("a_stopped_agents_details_say_stopped_once", () => {
    show(detailsAgent({ status: "stopped", stop_cause: "stopped" }));
    expect(screen.getAllByText("Stopped")).toHaveLength(1);
  });

  it("the_details_show_a_field_this_client_does_not_know", () => {
    const extra: UiSchema = { ...schema, card_fields: [...schema.card_fields, { key: "owner", label: "Owner", kind: "mono", show_when: { type: "always" }, value_labels: {}, value_tones: {} }] };
    show(detailsAgent(), extra);
    expect(screen.getByText("Owner")).toHaveClass("mono-label");
    expect(screen.getByText("antonio")).toBeInTheDocument();
  });

  it("panel_shows_the_model", () => {
    show(detailsAgent({ kind: "orchestrator", model: "opus" }));
    expect(screen.getByText("Model: opus")).toBeInTheDocument();
    show(detailsAgent({ id: "f", kind: "specialist" }));
    expect(screen.getByText("Model: persona default")).toBeInTheDocument();
  });

  it("a_solo_agent_shows_no_model", () => {
    show(detailsAgent({ model: "opus" }));
    expect(screen.queryByText(/^Model:/)).toBeNull();
  });

  it("panel_shows_the_mode_and_effort", () => {
    show(detailsAgent({ mode: "auto", effort: "high" }));
    expect(screen.getByText("Mode: auto · Effort: high")).toBeInTheDocument();
    show(detailsAgent({ id: "b", effort: "max" }));
    expect(screen.getByText("Effort: max")).toBeInTheDocument();
  });

  it("an_agent_that_predates_modes_shows_no_mode_line", () => {
    show(detailsAgent());
    expect(screen.queryByText(/Mode:|Effort:/)).toBeNull();
  });
});

describe("the Repos row", () => {
  it("the_repos_row_lists_workspaces_and_owned_ones_say_so", () => {
    render(<ReposRow agent={agent({ workspaces: [workspace("core"), { ...workspace("notes"), owned: true }] })} />);
    const row = screen.getByText("Repos").parentElement!;
    expect(screen.getByText("Repos")).toHaveClass("mono-label");
    expect(within(row).getByText("core")).toBeInTheDocument();
    expect(within(row).getByText("notes")).toHaveAttribute("title", "fleet/x · created in this task");
  });

  it("the_repos_row_offers_no_add_repo", () => {
    render(<ReposRow agent={agent()} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("workspace_tip_names_start_and_merge_route", () => {
    expect(workspaceTip({ ...workspace("core"), base: "abcdef1234", from_branch: "feat" })).toBe(
      "fleet/x, started from feat at abcdef1\n\nMerge fleet/x into feat first (a fast-forward if you did not commit there meanwhile), then merge feat as usual.",
    );
  });

  it("workspace_tip_for_a_url_repo_has_no_merge_note", () => {
    expect(workspaceTip({ ...workspace("core"), base: "abcdef1234", from_branch: "origin/main" })).toBe("fleet/x, started from origin/main at abcdef1");
  });

  it("workspace_tip_without_from_branch", () => {
    expect(workspaceTip({ ...workspace("core"), base: "abcdef1234" })).toBe("fleet/x, started at abcdef1");
    expect(workspaceTip({ ...workspace("core"), base: "abcdef1234", owned: true })).toBe("fleet/x · created in this task");
  });
});

describe("panel.css", () => {
  it("panel_css_uses_tokens_only", () => {
    const css = readFileSync("src/panel/panel.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });

  it("the_bottom_boxes_sit_in_the_panel_margin", () => {
    // Lite frames the reply and the repo request alike: Margin::same(panel::MARGIN).
    const css = ["src/panel/panel.css", "src/board/board.css"].map((f) => readFileSync(f, "utf8")).join("\n");
    for (const box of ["reply", "repo-request"]) {
      expect(css.match(new RegExp(`^\\.${box} \\{[^}]*\\bpadding: ([^;]+);`, "m"))?.[1], box).toBe("var(--panel-margin)");
    }
  });
});
