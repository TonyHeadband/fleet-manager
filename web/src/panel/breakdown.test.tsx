import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import type { BreakdownVersion } from "../api/generated/BreakdownVersion";
import type { FleetView } from "../api/generated/FleetView";
import type { GateStep } from "../api/generated/GateStep";
import type { Step } from "../api/generated/Step";
import { agent, form } from "../test/fixtures";
import { Breakdown } from "./Breakdown";

// Ported from crates/fleet-gui/tests/panels.rs (mod fleet); the names are Lite's.
const step = (n: number, title: string, state: Step["state"], agent: string | null = null, over: Partial<Step> = {}): Step => ({
  n, title, specialist: "rust", brief: "", depends_on: n > 1 ? [n - 1] : [], release_note: false, state, agent, ...over,
});
const version = (v: number, approval: BreakdownVersion["approval"], steps: Step[], comment: string | null = null): BreakdownVersion => ({ version: v, steps, approval, comment, proposed_at: 0 });
const gateStep = (name: string, command: string, exit_code: number | null, output_tail = "", duration_secs = 1): GateStep => ({ name, command, because: [], exit_code, duration_secs, output_tail, counted: false });
const viewOf = (breakdowns: BreakdownVersion[], gate_runs: FleetView["gate_runs"] = []): FleetView => ({ task: "o", breakdowns, gate_runs });
const agents = [agent({ id: "f1", goal: "fix redirect" })];

function show(view: FleetView, over: Partial<Parameters<typeof Breakdown>[0]> = {}) {
  const props = { view, agents, nicknames: {}, pending: false, busy: false, draft: "", onDraft: vi.fn(), onApprove: vi.fn(), onRequestChanges: vi.fn(), onOpen: vi.fn(), form: form("breakdown_decision"), ...over };
  const r = render(<Breakdown {...props} />);
  return { ...props, rerender: (more: Partial<typeof props>) => r.rerender(<Breakdown {...props} {...more} />) };
}

describe("Breakdown", () => {
  it("breakdown_panel_shows_the_latest_version_and_its_history", async () => {
    const { onOpen } = show(viewOf([version(1, "changes_requested", [step(1, "old plan", "pending")], "too big"), version(2, "proposed", [step(1, "add a test", "running", "f1")])]));
    const latest = screen.getAllByRole("table")[0]!;
    expect(within(latest).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["#", "Step", "Specialist", "Depends on", "Mode", "Effort", "State", "Agent"]);
    expect(screen.getByText("v2 · proposed")).toBeInTheDocument();
    expect(screen.getByText("add a test")).toBeInTheDocument();
    expect(screen.getByText("old plan"), "earlier versions wait behind History").not.toBeVisible();
    await userEvent.click(screen.getByText("History"));
    expect(screen.getByText("v1 · changes requested")).toBeVisible();
    expect(screen.getByText("Changes asked: too big")).toBeVisible();
    await userEvent.click(screen.getByRole("link", { name: "fix redirect" }));
    expect(onOpen).toHaveBeenCalledWith("f1");
  });

  it("the_steps_table_is_lites_grid", () => {
    show(viewOf([version(1, "approved", [step(1, "a long step title that wraps", "done")])]));
    for (const h of screen.getAllByRole("columnheader")) expect(h).toHaveClass("mono-label");
    // A table wider than the panel scrolls on its own, so the decision and the gate stay in place.
    expect(screen.getByRole("table").parentElement).toHaveClass("steps-scroll");
    // Only the step's title wraps; the other cells keep to one line, as Lite's grid does.
    const css = readFileSync("src/panel/panel.css", "utf8");
    expect(css).toMatch(/\.steps th, \.steps td \{[^}]*white-space: nowrap/);
    expect(css).toMatch(/\.steps td:nth-child\(2\) \{[^}]*white-space: normal/);
    // Lite's grid is striped with its faint background, the raised colour.
    expect(css).toMatch(/\.steps-scroll \{[^}]*overflow-x: auto/);
    expect(css, "the column holds to the panel's width, so the box can scroll").toMatch(/\.breakdown \{[^}]*grid-template-columns: minmax\(0, 1fr\)/);
    expect(css).toMatch(/\.steps tbody tr:nth-child\(odd\) \{[^}]*background: var\(--raised\)/);
  });

  it("breakdown_shows_each_steps_mode_and_effort", () => {
    show(viewOf([version(1, "approved", [step(1, "one", "done", null, { mode: "manual", effort: "max" }), step(2, "two", "pending", null, { depends_on: [] })])]));
    expect(screen.getByText("manual (stricter)")).toBeInTheDocument();
    expect(screen.getByText("max")).toBeInTheDocument();
    // Neither step depends on anything or has an agent; step 2 names neither mode nor effort.
    expect(screen.getAllByText("—")).toHaveLength(6);
  });

  it("breakdown_approve_and_request_changes", async () => {
    const draft = { value: "" };
    const { onApprove, onRequestChanges, rerender } = show(viewOf([version(2, "proposed", [step(1, "a", "pending")])]), { pending: true, onDraft: (t: string) => (draft.value = t) });
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onApprove).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Request changes" }), "a comment first").toBeDisabled();
    expect(screen.getByLabelText("Or ask for changes")).toHaveAttribute("placeholder", "What should change?");
    rerender({ pending: true, draft: "split step 2" });
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }));
    expect(onRequestChanges).toHaveBeenCalledWith("split step 2");
    rerender({ pending: true, busy: true, draft: "split step 2" });
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    rerender({ pending: false });
    expect(screen.queryByRole("button", { name: "Approve" }), "nothing to decide").toBeNull();
  });

  it("the_decision_follows_the_daemons_description", () => {
    const described = form("breakdown_decision");
    described.submit.label = "Looks good";
    show(viewOf([version(1, "proposed", [])]), { pending: true, form: described });
    expect(screen.getByRole("button", { name: "Looks good" })).toBeInTheDocument();
  });

  it("a_specialist_reads_its_personas_nickname", () => {
    show(viewOf([version(1, "approved", [step(1, "a", "done")])]), { nicknames: { rust: "Ferris" } });
    expect(screen.getByText("Ferris (rust)")).toBeInTheDocument();
  });

  it("gate_failure_shows_tail", async () => {
    const failed = { at: 0, base: "main", steps: [gateStep("test", "cargo test", 101, "test login::redirect ... FAILED")], unmatched: [], pr_url: "https://example.invalid/pr/1" };
    const passed = { at: 1, base: "main", steps: [gateStep("test", "cargo test", 0)], unmatched: [], pr_url: "https://example.invalid/pr/2" };
    const { rerender } = show(viewOf([version(1, "approved", [])], [failed]));
    expect(screen.getByText("test failed (exit 101)")).toBeInTheDocument();
    expect(screen.getByText(/redirect \.\.\. FAILED/)).not.toBeVisible();
    await userEvent.click(screen.getByText("Output"));
    expect(screen.getByText(/redirect \.\.\. FAILED/)).toBeVisible();
    rerender({ view: viewOf([version(1, "approved", [])], [failed, passed]) });
    expect(screen.getByText("Tests passed")).toBeInTheDocument();
    expect(screen.getAllByRole("link").map((l) => l.textContent), "only the last run's PR").toEqual(["https://example.invalid/pr/2"]);
    rerender({ view: viewOf([version(1, "approved", [])], [{ ...failed, steps: [gateStep("test", "cargo test", null)] }]) });
    expect(screen.getByText("test timed out")).toBeInTheDocument();
  });

  it("gate_lists_each_step", () => {
    const run = {
      at: 0, base: "main", unmatched: ["build.rs"], pr_url: null,
      steps: [gateStep("fmt", "cargo fmt --all --check", 0, "fmt says fine", 3), gateStep("gui", "cargo build -p fleet-gui\n  cargo test -p fleet-gui", 101, "gui test FAILED", 42)],
    };
    show(viewOf([version(1, "approved", [])], [run]));
    expect(screen.getByText("gui failed (exit 101)")).toBeInTheDocument();
    expect(screen.getByText("fmt · cargo fmt --all --check · 3s · ok")).toBeInTheDocument();
    expect(screen.getByText("gui · cargo build -p fleet-gui cargo test -p fleet-gui · 42s · exit 101")).toBeInTheDocument();
    expect(screen.getByText("Every step ran: no step covers build.rs")).toBeInTheDocument();
    expect(within(screen.getByText("Output").closest("details")!).queryByText(/fmt says fine/), "only the failed step's tail").toBeNull();
  });

  it("no_breakdown_yet_and_no_gate_run_say_so", () => {
    show(viewOf([]));
    expect(screen.getByText("No breakdown proposed yet.")).toBeInTheDocument();
    expect(screen.getByText("The tests run when the orchestrator opens the PR.")).toBeInTheDocument();
  });
});
