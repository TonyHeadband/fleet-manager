import { title } from "../api/schema";
import type { Agent } from "../api/generated/Agent";
import type { BreakdownVersion } from "../api/generated/BreakdownVersion";
import type { FleetView } from "../api/generated/FleetView";
import type { FormDef } from "../api/generated/FormDef";
import { inputs } from "../api/forms";
import { specialistLabel } from "../board/groups";
import { prUrl } from "./fleetViews";
import { Button } from "../ui/Button";
import { approvalText, failedStep, gateText, stateText, stepLine } from "./breakdown";
import { Avatar } from "../ui/Avatar";

interface Props {
  view: FleetView;
  /** The task's agents, which the step table names. */
  agents: Agent[];
  /** The orchestrator's persona's names for its specialists. */
  nicknames: Record<string, string>;
  /** The newest version waits on a decision: Approve and Request changes show. */
  pending: boolean;
  /** A decision is on its way, or sent and not yet in the view: the controls stay, greyed. */
  busy: boolean;
  draft: string;
  onDraft: (text: string) => void;
  onApprove: () => void;
  onRequestChanges: (comment: string) => void;
  onOpen: (agentId: string) => void;
  /** The daemon's `breakdown_decision`, which names the actions and the comment box. */
  form: FormDef;
}

const HEADERS = ["#", "Step", "Specialist", "Depends on", "Mode", "Effort", "State", "Agent"];

/**
 * Lite's `breakdown_panel_deciding`: the latest version and its steps, earlier versions behind History, the
 * decision while one is pending, then the last test gate run.
 */
export function Breakdown({ view, agents, nicknames, pending, busy, draft, onDraft, onApprove, onRequestChanges, onOpen, form }: Props) {
  const latest = view.breakdowns.at(-1);
  if (!latest) {
    return (
      <div className="breakdown">
        <p className="note">No breakdown proposed yet.</p>
        <Gate view={view} />
      </div>
    );
  }
  const earlier = view.breakdowns.slice(0, -1).reverse();
  const approve = form.submit.id === "approve" ? form.submit : form.more.find((s) => s.id === "approve");
  const changes = [form.submit, ...form.more].find((s) => s.id === "request_changes");
  const comment = inputs(form).find((i) => i.key === "comment");
  const ready = !!changes && changes.needs.every((k) => k !== "comment" || draft.trim() !== "");

  return (
    <div className="breakdown">
      <Version v={latest} agents={agents} nicknames={nicknames} onOpen={onOpen} />
      {earlier.length > 0 && (
        <details className="history">
          <summary>History</summary>
          {earlier.map((v) => (
            <Version key={v.version} v={v} agents={agents} nicknames={nicknames} onOpen={onOpen} />
          ))}
        </details>
      )}
      {pending && (
        <div className="decide">
          {approve && (
            <Button variant="primary" disabled={busy} onClick={onApprove}>
              {approve.label}
            </Button>
          )}
          {changes && comment && (
            <>
              <label className="decide-comment">
                <span className="mono-label">{comment.label}</span>
                <textarea rows={3} placeholder={comment.placeholder ?? undefined} value={draft} disabled={busy} onChange={(e) => onDraft(e.target.value)} />
              </label>
              <Button disabled={busy || !ready} onClick={() => onRequestChanges(draft.trim())}>
                {changes.label}
              </Button>
            </>
          )}
        </div>
      )}
      <hr className="breakdown-rule" />
      <Gate view={view} />
    </div>
  );
}

function Version({ v, agents, nicknames, onOpen }: { v: BreakdownVersion; agents: Agent[]; nicknames: Record<string, string>; onOpen: (id: string) => void }) {
  return (
    <section className="breakdown-version">
      <p className="version-line">{`v${v.version} · ${approvalText(v.approval)}`}</p>
      {v.comment && <p className="note">{`Changes asked: ${v.comment}`}</p>}
      <div className="steps-scroll">
        <table className="steps">
          <thead>
            <tr>
              {HEADERS.map((h) => (
                <th key={h} className="mono-label">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {v.steps.map((s) => {
              const state = stateText(s.state);
              const agent = s.agent ? agents.find((a) => a.id === s.agent) : undefined;
              return (
                <tr key={s.n}>
                  <td>{s.n}</td>
                  <td>{s.title}</td>
                  <td>
                    <span className="step-specialist">
                      <Avatar kind="specialist" seed={s.specialist} title={s.specialist} />
                      {specialistLabel(nicknames[s.specialist], s.specialist)}
                    </span>
                  </td>
                  <td>{s.depends_on.length > 0 ? s.depends_on.join(", ") : "—"}</td>
                  {/* The daemon refuses a step that loosens its specialist's mode, so a mode named here tightens it. */}
                  <td>{s.mode ? `${s.mode} (stricter)` : "—"}</td>
                  <td>{s.effort ?? "—"}</td>
                  <td>
                    <span className="step-state">
                      <span className="dot" data-tone={state.tone} aria-hidden="true" />
                      {state.word}
                    </span>
                  </td>
                  <td>
                    {s.agent ? (
                      <a
                        href="#"
                        onClick={(e) => {
                          e.preventDefault();
                          onOpen(s.agent!);
                        }}
                      >
                        {agent ? title(agent) : s.agent}
                      </a>
                    ) : (
                      "—"
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** The last gate run: its result, one line per step it ran, the failed step's output tail folded, and its PR. */
function Gate({ view }: { view: FleetView }) {
  const run = view.gate_runs.at(-1);
  if (!run) return <p className="note">The tests run when the orchestrator opens the PR.</p>;
  const head = gateText(run);
  const failed = failedStep(run);
  const pr = prUrl(view);
  return (
    <section className="gate">
      <p className="gate-head">
        <span className="dot" data-tone={head.tone} aria-hidden="true" />
        {head.text}
      </p>
      {run.steps.map((s, i) => (
        <p key={i} className="gate-step">
          {stepLine(s)}
        </p>
      ))}
      {run.unmatched.length > 0 && <p className="note">{`Every step ran: no step covers ${run.unmatched.join(", ")}`}</p>}
      {failed && failed.output_tail !== "" && (
        <details key={run.at} className="gate-output">
          <summary>Output</summary>
          <pre className="pane">{failed.output_tail}</pre>
        </details>
      )}
      {pr && (
        <p className="gate-pr">
          <span className="mono-label">PR</span>
          <a href={pr} target="_blank" rel="noreferrer">
            {pr}
          </a>
        </p>
      )}
    </section>
  );
}
