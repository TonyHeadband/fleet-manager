import { useEffect, useId, useRef, type KeyboardEvent } from "react";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import type { RepoInfo } from "../api/generated/RepoInfo";
import { personaLabels } from "../board/filters";
import { newTask } from "../generated/layout";
import { appNative, copyText } from "../native/native";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { Chip } from "../ui/Chip";
import { SmallIconButton } from "../ui/SmallIconButton";
import { Select } from "../ui/Select";
import {
  checkoutLabel, fleetBlocker, openReleases, personaBlocker, startBlocker, startOf, toggleRepo, toolErrorText, wantsLocal, workflowLine,
  type Draft, type Listings,
} from "./model";

/** Links to views that arrive in later steps show in place, disabled, as the board's do. */
const NOT_YET = "not on the web yet";
/** The ARIA radio pattern: these keys move to the next or previous radio, wrapping, and choose it. */
const ARROW_STEP: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

interface Props {
  draft: Draft;
  onDraft: (d: Draft) => void;
  personas: Persona[];
  product: Product | undefined;
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
 * Lite's New task dialog (`new_task_dialog`): the goal, title, persona and release, the task's repo and where it
 * starts, its workflow, the daemon's refusal, and Start. Every task is a direct orchestrator: no type to pick. Closing
 * by any route keeps what was typed.
 */
export function NewTaskDialog({ draft, onDraft, personas, product, repos, reposError, listings, onPersona, onRetry, onStart, onClose, onWizard }: Props) {
  const ids = { goal: useId(), title: useId(), persona: useId(), release: useId(), repos: useId() };
  const goal = useRef<HTMLTextAreaElement>(null);
  // The goal takes the focus on each opening only.
  useEffect(() => goal.current?.focus(), []);

  const ready = personas.filter((p) => p.ready);
  const labels = personaLabels(ready, ready.map((p) => p.id));
  const blocked = personaBlocker(personas, draft.persona);
  const why = blocked ?? fleetBlocker(draft) ?? startBlocker(repos, draft, listings);
  const canStart = !draft.busy && draft.prompt.trim() !== "" && why === null;
  const start = () => canStart && onStart();

  // One tab stop in the repo radios: the chosen one, else the first.
  const radios = useRef<HTMLDivElement>(null);
  const tabStop = Math.max(0, repos.findIndex((r) => draft.chosen.includes(r.name)));
  const stepRepo = (e: KeyboardEvent<HTMLDivElement>) => {
    const by = ARROW_STEP[e.key];
    if (by === undefined || repos.length === 0) return;
    e.preventDefault();
    const buttons = [...(radios.current?.querySelectorAll<HTMLElement>('[role="radio"]') ?? [])];
    const from = buttons.indexOf(document.activeElement as HTMLElement);
    const to = ((from < 0 ? tabStop : from) + by + repos.length) % repos.length;
    const repo = repos[to];
    if (!repo) return;
    onDraft({ ...draft, chosen: [repo.name] });
    buttons[to]?.focus();
  };

  return (
    <Modal label="New task" className="dialog new-task" onClose={onClose}>
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

      <label className="caption" htmlFor={ids.persona}>Persona</label>
      <Select
        id={ids.persona}
        value={ready.some((p) => p.id === draft.persona) ? draft.persona : ""}
        placeholder="Pick a persona"
        options={ready.map((p) => ({ value: p.id, label: labels.get(p.id) ?? p.id }))}
        onChange={onPersona}
      />
      {blocked && <p className="danger">{blocked}</p>}

      <label className="caption" htmlFor={ids.release}>Release</label>
      <Select
        id={ids.release}
        value={draft.release === null || draft.release === undefined ? "" : String(draft.release)}
        options={[{ value: "", label: "Backlog" }, ...openReleases(product).map((r) => ({ value: String(r.id), label: r.version }))]}
        onChange={(v) => onDraft({ ...draft, release: v === "" ? null : Number(v) })}
      />

      <div className="new-task-repos" role="radiogroup" aria-labelledby={ids.repos}>
        <span className="caption" id={ids.repos}>Repo</span>
        {repos.length === 0 ? (
          <p className="note">This product has no repos; add them in Products</p>
        ) : (
          <div className="chips" ref={radios} onKeyDown={stepRepo}>
            {repos.map((r, i) => (
              <Chip key={r.name} radio tabIndex={i === tabStop ? 0 : -1} selected={draft.chosen.includes(r.name)} onClick={() => onDraft(toggleRepo(draft, r.name))}>
                {r.name}
              </Chip>
            ))}
          </div>
        )}
      </div>
      {reposError && <p className="danger">{reposError}</p>}

      <Starts draft={draft} onDraft={onDraft} repos={repos} listings={listings} onRetry={onRetry} />
      <Fleet draft={draft} repos={repos} />

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
    </Modal>
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
          // Named by its repo: two picks with a choice would otherwise give two "From URL" buttons alike.
          <div key={name} className="new-task-start" role="group" aria-label={`${name} starts`}>
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
                // A start the listing lacks (no main row) shows as its raw path, as Lite's, not as the first row.
                <Select
                  label={`Checkout of ${name}`}
                  value={current}
                  options={listing.checkouts.map((c) => ({ value: c.path, label: checkoutLabel(c) }))}
                  onChange={(v) => onDraft({ ...draft, starts: { ...draft.starts, [name]: v } })}
                />
              ))}
            {info.url === null && <p className="note">No URL: no pull request or merge tracking for this repo.</p>}
          </div>
        );
      })}
    </>
  );
}

/** The task's repo workflow (`fleet_section`). Its specialists are every one the product does not hide. */
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
