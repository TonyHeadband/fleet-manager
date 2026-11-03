import type { Approval } from "../api/generated/Approval";
import type { FormDef } from "../api/generated/FormDef";
import type { SubmitDef } from "../api/generated/SubmitDef";
import type { FleetView } from "../api/generated/FleetView";
import type { GateRun } from "../api/generated/GateRun";
import type { GateStep } from "../api/generated/GateStep";
import type { StepState } from "../api/generated/StepState";
import type { Tone } from "../api/generated/Tone";

/** The newest version waits on the owner's decision. */
export function pending(view: FleetView): boolean {
  return view.breakdowns.at(-1)?.approval === "proposed";
}

export function approvalText(a: Approval): string {
  return { proposed: "proposed", approved: "approved", changes_requested: "changes requested" }[a];
}

const STATES: Record<StepState, { word: string; tone: Tone }> = {
  pending: { word: "pending", tone: "neutral" },
  running: { word: "running", tone: "active" },
  done: { word: "done", tone: "done" },
  failed: { word: "failed", tone: "danger" },
};

export const stateText = (s: StepState) => STATES[s];

/** The step the run stopped at: the first that did not exit 0. */
export const failedStep = (run: GateRun) => run.steps.find((s) => s.exit_code !== 0);

/** Lite's `gate_text`: the run's headline and its tone. */
export function gateText(run: GateRun): { text: string; tone: Tone } {
  if (run.steps.length > 0 && !failedStep(run)) return { text: "Tests passed", tone: "done" };
  const s = failedStep(run);
  const text = !s ? "No step ran" : s.exit_code === null ? `${s.name} timed out` : `${s.name} failed (exit ${s.exit_code})`;
  return { text, tone: "danger" };
}

/** One line per step; a multi-line `run:` is collapsed so it stays one line. */
export function stepLine(s: GateStep): string {
  const result = s.exit_code === 0 ? "ok" : s.exit_code === null ? "timed out" : `exit ${s.exit_code}`;
  const command = s.command.split(/\s+/).filter(Boolean).join(" ");
  return `${s.name} · ${command} · ${s.duration_secs}s · ${result}`;
}

/** This build's copy of the daemon's `breakdown_decision`, used when the daemon's holds something it cannot draw. */
export const BREAKDOWN_DECISION: FormDef = (() => {
  const decide = (id: string, label: string, approve: boolean, needs: string[]): SubmitDef => ({
    id, label, method: "POST", path: "/api/tasks/{id}/breakdown/decide", fixed: { approve }, context: ["version"], needs,
  });
  return {
    id: "breakdown_decision",
    title: "Breakdown",
    submit: decide("approve", "Approve", true, []),
    items: [{ type: "field", key: "comment", label: "Or ask for changes", kind: "multiline", placeholder: "What should change?", default: null, rules: [], help: null, source: null }],
    more: [decide("request_changes", "Request changes", false, ["comment"])],
  };
})();
