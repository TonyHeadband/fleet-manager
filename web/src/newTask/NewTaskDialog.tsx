import { useEffect, useId, useRef } from "react";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import type { RepoInfo } from "../api/generated/RepoInfo";
import { personaLabels } from "../board/filters";
import { newTask } from "../generated/layout";
import { appNative, copyText } from "../native/native";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { SmallIconButton } from "../ui/SmallIconButton";
import {
  checkoutLabel, fleetBlocker, keepOneRepo, openReleases, personaBlocker, setType, startBlocker, startOf, toolErrorText, wantsLocal, workflowLine,
  type Draft, type Listings,
} from "./model";

/** Links to views that arrive in later steps show in place, disabled, as the board's do. */
const NOT_YET = "not on the web yet";

interface Props {
  draft: Draft;
  onDraft: (d: Draft) => void;
  personas: Persona[];
  product: Product | undefined;
  /** The orchestrator types that load, in the daemon's order. */
  orchestrators: string[];
  /** The product's repos, as last fetched for the dialog's persona. */
  repos: RepoInfo[];
  reposError: string | null;
  listings: Listings;
  onPersona: (id: string) => void;
  onRetry: (name: string, path: string) => void;
  onStart: () => void;
  onClose: () => void;
  /** Open the setup wizard, under a tool refusal: the app closes the dialog and opens the wizard. */
  onWizard: () => void;
}

/**
 * Lite's New task dialog (`new_task_dialog`): the goal, title, type, persona and release, the repos and where each
 * starts, an orchestrated task's workflow, the daemon's refusal, and Start. Closing by any route keeps what was typed.
 */
export function NewTaskDialog({ draft, onDraft, personas, product, orchestrators, repos, reposError, listings, onPersona, onRetry, onStart, onClose, onWizard }: Props) {
  const ids = { goal: useId(), title: useId(), type: useId(), persona: useId(), release: useId(), repos: useId() };
  const goal = useRef<HTMLTextAreaElement>(null);
  // The goal takes the focus on each opening only.
  useEffect(() => goal.current?.focus(), []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  const orchestrated = draft.role !== null;
  const ready = personas.filter((p) => p.ready);
  const labels = personaLabels(ready, ready.map((p) => p.id));
  const blocked = personaBlocker(personas, draft.persona);
  const why = blocked ?? fleetBlocker(draft) ?? startBlocker(repos, draft, listings);
  const canStart = !draft.busy && draft.prompt.trim() !== "" && why === null;
  const start = () => canStart && onStart();

  const toggle = (name: string) => {
    const before = draft.chosen;
    const next = { ...draft, chosen: before.includes(name) ? before.filter((c) => c !== name) : [...before, name] };
    onDraft(orchestrated ? keepOneRepo(next, before) : next);
  };

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog new-task" role="dialog" aria-modal="true" aria-label="New task">
        <div className="new-task-head">
          <h2>New task</h2>
          <SmallIconButton icon="close" label="Close" onClick={onClose} />
        </div>

        <label className="caption" htmlFor={ids.goal}>Goal</label>
        <textarea
          id={ids.goal}
          ref={goal}
          rows={newTask.promptRows}
          value={draft.prompt}
          onChange={(e) => onDraft({ ...draft, prompt: e.target.value })}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || !(e.ctrlKey || e.metaKey)) return;
            e.preventDefault();
            start();
          }}
        />

        <label className="caption" htmlFor={ids.title}>Title</label>
        <input id={ids.title} type="text" placeholder="Optional: the agent names the task when this is empty" value={draft.title} onChange={(e) => onDraft({ ...draft, title: e.target.value })} />

        <label className="caption" htmlFor={ids.type}>Type</label>
        <select id={ids.type} value={draft.role ?? ""} onChange={(e) => onDraft(setType(draft, e.target.value === "" ? null : e.target.value))}>
          <option value="">Solo</option>
          {orchestrators.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>

        <label className="caption" htmlFor={ids.persona}>Persona</label>
        <select id={ids.persona} value={ready.some((p) => p.id === draft.persona) ? draft.persona : ""} onChange={(e) => onPersona(e.target.value)}>
          {!ready.some((p) => p.id === draft.persona) && <option value="">Pick a persona</option>}
          {ready.map((p) => (
            <option key={p.id} value={p.id}>
              {labels.get(p.id)}
            </option>
          ))}
        </select>
        {blocked && <p className="danger">{blocked}</p>}

        <label className="caption" htmlFor={ids.release}>Release</label>
        <select id={ids.release} value={draft.release ?? ""} onChange={(e) => onDraft({ ...draft, release: e.target.value === "" ? null : Number(e.target.value) })}>
          <option value="">Backlog</option>
          {openReleases(product).map((r) => (
            <option key={r.id} value={r.id}>
              {r.version}
            </option>
          ))}
        </select>

        <div className="new-task-repos" role="group" aria-labelledby={ids.repos}>
          <span className="caption" id={ids.repos}>{orchestrated ? "Repo" : "Repos (optional)"}</span>
          {repos.length === 0 ? (
            <p className="note">This product has no repos; add them in Products</p>
          ) : (
            <div className="chips">
              {repos.map((r) => (
                <Chip key={r.name} selected={draft.chosen.includes(r.name)} onClick={() => toggle(r.name)}>
                  {r.name}
                </Chip>
              ))}
            </div>
          )}
        </div>
        {draft.dropped && <p className="note">{draft.dropped}</p>}
        {reposError && <p className="danger">{reposError}</p>}

        <Starts draft={draft} onDraft={onDraft} repos={repos} listings={listings} onRetry={onRetry} />
        {orchestrated && <Fleet draft={draft} repos={repos} />}

        {draft.refusal ? (
          <div className="new-task-refusal">
            <p>{toolErrorText(draft.refusal)}</p>
            {"fix" in draft.refusal ? (
              <>
                {draft.refusal.fix.install && (
                  <div className="new-task-command">
                    <code>{draft.refusal.fix.install}</code>
                    <Button onClick={() => void copyText(appNative(), draft.refusal && "fix" in draft.refusal ? (draft.refusal.fix.install ?? "") : "")}>Copy</Button>
                  </div>
                )}
                <a href={draft.refusal.fix.link} target="_blank" rel="noreferrer">
                  {draft.refusal.fix.link}
                </a>
                <Button onClick={onWizard}>Open the setup wizard</Button>
              </>
            ) : (
              <p className="note">Log in again from the persona's setup.</p>
            )}
          </div>
        ) : (
          draft.error && <p className="danger">{draft.error}</p>
        )}

        <div className="new-task-actions">
          <span className="reply-hint">Ctrl+Enter</span>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!canStart} title={why ?? undefined} onClick={start}>
            Start task
          </Button>
        </div>
      </div>
    </div>
  );
}

/** For each picked repo the product knows by a path: where the task starts (`starts_section`). */
function Starts({ draft, onDraft, repos, listings, onRetry }: Pick<Props, "draft" | "onDraft" | "repos" | "listings" | "onRetry">) {
  return (
    <>
      {draft.chosen.map((name) => {
        const info = repos.find((r) => r.name === name);
        if (!info?.path) return null;
        const path = info.path;
        const local = wantsLocal(repos, draft, name);
        const listing = listings[path];
        const current = startOf(repos, draft, listings, name) ?? "";
        return (
          <div key={name} className="new-task-start">
            <div className="chips">
              <span className="note">{`${name} starts`}</span>
              {info.url !== null && (
                <>
                  <Chip selected={!local} onClick={() => onDraft({ ...draft, starts: { ...draft.starts, [name]: null } })}>
                    From URL
                  </Chip>
                  {/* The repo's own path stands for its main checkout until the listing says which that is. */}
                  <Chip selected={local} onClick={() => !local && onDraft({ ...draft, starts: { ...draft.starts, [name]: path } })}>
                    Local checkout
                  </Chip>
                </>
              )}
            </div>
            {local &&
              (!listing || listing.state === "loading" ? (
                <p className="note">Loading checkouts…</p>
              ) : listing.state === "failed" ? (
                <p className="danger">
                  {`Cannot list the checkouts of ${name}: ${listing.error}`}{" "}
                  <button type="button" className="link" onClick={() => onRetry(name, path)}>
                    Retry
                  </button>
                </p>
              ) : (
                <select aria-label={`Checkout of ${name}`} value={current} onChange={(e) => onDraft({ ...draft, starts: { ...draft.starts, [name]: e.target.value } })}>
                  {listing.checkouts.map((c) => (
                    <option key={c.path} value={c.path}>
                      {checkoutLabel(c)}
                    </option>
                  ))}
                </select>
              ))}
            {info.url === null && <p className="note">No URL: no pull request or merge tracking for this repo.</p>}
          </div>
        );
      })}
    </>
  );
}

/** An orchestrated task's repo workflow (`fleet_section`). Its specialists are every one the product does not hide. */
function Fleet({ draft, repos }: Pick<Props, "draft" | "repos">) {
  const name = draft.chosen[0];
  if (name === undefined) return null;
  const repo = repos.find((r) => r.name === name);
  if (!repo) return <p className="note">The workflow is checked when the task is created.</p>;
  const line = workflowLine(repo.name, repo.workflow);
  return line.blocks ? (
    <div className="new-task-fleet">
      <p className="danger">{line.text}</p>
      <Button disabled title={NOT_YET}>
        Create workflow
      </Button>
    </div>
  ) : (
    <p className="new-task-checks">{line.text}</p>
  );
}
