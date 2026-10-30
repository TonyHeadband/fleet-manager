import { Fragment, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import type { FleetClient } from "../api/client";
import type { Orphan } from "../api/generated/Orphan";
import type { OrphanKind } from "../api/generated/OrphanKind";
import type { PrState } from "../api/generated/PrState";
import type { Product } from "../api/generated/Product";
import type { Task } from "../api/generated/Task";
import type { Tone } from "../api/generated/Tone";
import { formatDuration } from "../api/schema";
import { DiffContent, screenText, type Diff } from "../components/PreviewPane";
import { formatBytes } from "../shell/counts";
import type { Notice } from "../shell/messages";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Section } from "../ui/Section";
import { progressText, taskOutcome, type Entry, type Live } from "./model";

// A port of crates/fleet-gui/src/components/passport.rs: the right side of the Tasks view.

const ago = (now: number, at: number) => `${formatDuration(now - at)} ago`;

/** Why Archive is refused while a PR is open, in the daemon's words. */
function openPrReason(t: Task): string | null {
  const open = (t.prs ?? []).find((p) => p.state === "open");
  return open ? `${open.url} is still open; merge or close it before archiving` : null;
}

const PR_STATE: Record<PrState, string> = { open: "Open", merged: "Merged", closed: "Closed" };

const KIND_TEXT: Record<OrphanKind, string> = {
  task_folder: "A folder under the tasks root with no task record",
  agent_folder: "A daemon agent folder with no task record",
  worktree: "A worktree under the tasks root with no task record",
};

interface Props {
  entry: Entry;
  badge: [string, Tone | undefined];
  client: FleetClient;
  now: number;
  live: Record<string, Live>;
  /** The active product as loaded; undefined while the list is out or with none picked, so its repos are unknown. */
  product: Product | undefined;
  onArchive: () => void;
  onStop: () => void;
  onOpenOnBoard: () => void;
  onRefresh: () => void;
  /** The task changed: the list is asked for again. */
  onChanged: () => void;
  say: (notice: Notice, text: string) => void;
}

/** Keyed by entry: another task starts with no Confirm Stop pending and its own diff. */
export function Passport(props: Props) {
  const { entry } = props;
  return (
    <div className="passport">
      {entry.kind === "task" ? (
        <TaskPassport {...props} task={entry.task} />
      ) : (
        <OrphanPassport orphan={entry.orphan} badge={props.badge} onArchive={props.onArchive} />
      )}
    </div>
  );
}

function TaskPassport({
  task: t,
  badge,
  client,
  now,
  live,
  product,
  onArchive,
  onStop,
  onOpenOnBoard,
  onRefresh,
  onChanged,
  say,
}: Props & { task: Task }) {
  const [confirmStop, setConfirmStop] = useState(false);
  const archived = t.state === "archived";
  const stopped = t.state === "stopped";
  const outcome = taskOutcome(t);
  const diff = useDiff(client, t.id, stopped);
  // A task with no PR to merge can be marked done; with the product's repos unknown, nobody can tell yet.
  const canMarkDone =
    !archived &&
    !t.done_at &&
    !t.done_by_hand &&
    product !== undefined &&
    !t.repos.some((w) => product.repos.some((r) => r.name === w.repo && r.url !== null));
  const change = (call: () => Promise<unknown>) =>
    void call()
      .catch((e: unknown) => say("error", `Could not update ${t.name}: ${e instanceof Error ? e.message : String(e)}`))
      .finally(onChanged);
  const blocked = openPrReason(t);

  return (
    <>
      <TitleRow name={t.name} badge={badge} tip="State">
        {/* An archived passport is read-only, and its task has left the board. */}
        {!archived && (
          <>
            {canMarkDone && <Button onClick={() => change(() => client.markDone(t.id))}>Mark done</Button>}
            <Button className="ghost" onClick={onOpenOnBoard}>
              Open on board
            </Button>
            {t.state === "running" &&
              live[t.id]?.canStop &&
              (confirmStop ? (
                <Button
                  variant="danger"
                  onClick={() => {
                    setConfirmStop(false);
                    onStop();
                  }}
                >
                  Confirm Stop
                </Button>
              ) : (
                <Button onClick={() => setConfirmStop(true)}>Stop</Button>
              ))}
            <Button
              variant="quiet-danger"
              disabled={!stopped || blocked !== null}
              title={blocked ?? (stopped ? undefined : "stop it first")}
              onClick={onArchive}
            >
              Archive…
            </Button>
          </>
        )}
      </TitleRow>
      <p className="passport-meta low">
        {t.created_at !== null ? `started ${ago(now, t.created_at)}` : "started: unknown"}
        {t.archived_at !== null && ` · archived ${ago(now, t.archived_at)}`}
      </p>
      <code className="passport-path">{t.workdir}</code>
      <div className="passport-release">
        <span className="low">Release</span>
        <ReleaseCombo
          task={t}
          product={product}
          locked={outcome === "merged" || outcome === "done_by_hand"}
          onPick={(to) => change(() => client.setTaskRelease(t.id, to))}
        />
        <span className="low">· Progress</span>
        <span>{progressText(t, live)}</span>
      </div>

      <Section title="Goal">
        <p className="passport-goal">{t.goal.split("\n")[0]}</p>
      </Section>

      <Section title="Repos">
        {t.repos.length === 0 ? (
          <p className="low">No repo</p>
        ) : (
          <div className="passport-grid" data-columns="3">
            {t.repos.map((w) => (
              <Fragment key={w.repo}>
                <span className="high">{w.repo}</span>
                <code>{w.branch}</code>
                <span className="low">
                  {w.owned
                    ? "created in this task"
                    : w.from_branch
                      ? `from ${w.from_branch} at ${w.base.slice(0, 7)}`
                      : `from ${w.base.slice(0, 7)}`}
                </span>
              </Fragment>
            ))}
          </div>
        )}
      </Section>

      <Section title="Pull requests">
        {(t.prs ?? []).length === 0 && <p className="low">None found</p>}
        {(t.prs ?? []).map((r) => (
          <div key={r.url} className="passport-pr">
            <span className="high">{r.repo}</span>
            <code>{r.url}</code>
            <span className="low">{PR_STATE[r.state]}</span>
          </div>
        ))}
      </Section>

      <Section title="Footprint">
        {t.footprint && (
          <div className="passport-grid" data-columns="3">
            {t.footprint.items.map((item) => (
              <Fragment key={item.path}>
                <span>{item.label}</span>
                <code>{item.path}</code>
                <span>{formatBytes(item.bytes)}</span>
                {item.build_bytes !== null && (
                  <>
                    <span className="low">of which target/</span>
                    <span />
                    <span className="low">{formatBytes(item.build_bytes)}</span>
                  </>
                )}
              </Fragment>
            ))}
            <span className="high">Total</span>
            <span />
            <span className="high">{formatBytes(t.footprint.total)}</span>
          </div>
        )}
        <div className="passport-line">
          <span className="low">{t.footprint ? `measured ${ago(now, t.footprint.measured_at)}` : "not measured yet"}</span>
          {!archived && (
            <Button className="ghost" onClick={onRefresh}>
              Refresh
            </Button>
          )}
        </div>
      </Section>

      <Section title="Activity">
        <div className="passport-grid" data-columns="2">
          <span className="low">Agents used</span>
          <span>{t.agents_used}</span>
          <span className="low">Time running</span>
          <span>{t.created_at !== null ? formatDuration((t.stopped_at ?? t.archived_at ?? now) - t.created_at) : "unknown"}</span>
          <span className="low">Time per status</span>
          {/* Counted from the version that started keeping them; a record made on upgrade has none. */}
          <span>
            {t.created_at === null && Object.values(t.times).every((v) => v === 0)
              ? "unknown"
              : `Working ${formatDuration(t.times.working)} · Needs you ${formatDuration(t.times.needs_you)} · Review ${formatDuration(t.times.review)} · Queued ${formatDuration(t.times.queued)}`}
          </span>
          <span className="low">Tokens · cost</span>
          <span className="low">not tracked yet</span>
        </div>
      </Section>

      <Section title="Log">
        {t.log.length === 0 && <p className="low">Nothing logged</p>}
        <div className="passport-grid passport-log" data-columns="2">
          {t.log.map((e, i) => (
            <Fragment key={i}>
              <span className="low">{ago(now, e.at)}</span>
              <span>{e.text}</span>
            </Fragment>
          ))}
        </div>
      </Section>

      {/* The worktrees are on disk until Archive; an archived task has nothing left to diff. */}
      {stopped && (
        <>
          <Section title="Diff">
            <Button className="ghost" aria-label="Reload the diff" title="Reload the diff" onClick={diff.load}>
              Refresh
            </Button>
            <div className="passport-box">
              <DiffContent diff={diff.state} />
            </div>
          </Section>
          <Section title="Last screen">
            {/* Only Stop saves one: an exit or a crash leaves no session to read. */}
            {t.last_screen !== null ? (
              <div className="passport-box">
                <pre className="pane">{screenText(t.last_screen)}</pre>
              </div>
            ) : (
              <p className="low">not saved</p>
            )}
          </Section>
        </>
      )}
    </>
  );
}

/** The orphan's passport: what it is, where, its worktrees and size, and Archive…. */
function OrphanPassport({ orphan: o, badge, onArchive }: { orphan: Orphan; badge: [string, Tone | undefined]; onArchive: () => void }) {
  return (
    <>
      <TitleRow name={o.path.split("/").filter(Boolean).pop() ?? o.path} badge={badge} tip="Found with no task record">
        <Button variant="quiet-danger" onClick={onArchive}>
          Archive…
        </Button>
      </TitleRow>
      <p className="passport-meta low">{KIND_TEXT[o.kind]}</p>
      <code className="passport-path">{o.path}</code>
      {o.worktrees.length > 0 && (
        <Section title="Worktrees">
          {o.worktrees.map((w) => (
            <code key={w} className="passport-path">
              {w}
            </code>
          ))}
        </Section>
      )}
      <Section title="Footprint">
        {o.bytes !== null ? <span>{formatBytes(o.bytes)}</span> : <span className="low">measuring…</span>}
      </Section>
    </>
  );
}

/** Name and badge, with the actions at the right end, in sight however long the passport is. */
function TitleRow({ name, badge, tip, children }: { name: string; badge: [string, Tone | undefined]; tip: string; children: ReactNode }) {
  return (
    <div className="passport-title">
      <h2>{name}</h2>
      <Badge label={tip} tone={badge[1]}>
        {badge[0]}
      </Badge>
      <span className="passport-actions">{children}</span>
    </div>
  );
}

/**
 * The task's release as a dropdown (`release_combo`): the unreleased ones, the task's own release even when released
 * (a released one takes no more tasks), then Backlog.
 */
function ReleaseCombo({
  task,
  product,
  locked,
  onPick,
}: {
  task: Task;
  product: Product | undefined;
  locked: boolean;
  onPick: (to: number | null) => void;
}) {
  const releases = product?.releases ?? [];
  const listed = releases.filter((r) => r.released_at === null || task.release === r.id);
  const own = task.release !== null && !listed.some((r) => r.id === task.release);
  return (
    <select
      aria-label="Release"
      value={task.release === null ? "" : String(task.release)}
      disabled={locked}
      title={locked ? "A finished task stays in its release" : undefined}
      onChange={(e) => {
        const to = e.target.value === "" ? null : Number(e.target.value);
        if (to !== task.release) onPick(to);
      }}
    >
      {own && <option value={String(task.release)}>Release</option>}
      {listed.map((r) => (
        <option key={r.id} value={String(r.id)}>
          {r.released_at !== null ? `${r.version} (released)` : r.version}
        </option>
      ))}
      <option value="">Backlog</option>
    </select>
  );
}

/** A stopped task's diff: loaded with its passport and again on Refresh; only the latest answer shows. */
function useDiff(client: FleetClient, id: string, stopped: boolean): { state: Diff; load: () => void } {
  const [state, setState] = useState<Diff>({ state: "none" });
  const asked = useRef(0);
  const load = useCallback(() => {
    const n = ++asked.current;
    setState({ state: "loading" });
    client.diff(id).then(
      (report) => n === asked.current && setState({ state: "loaded", report }),
      (e: unknown) => n === asked.current && setState({ state: "failed", message: e instanceof Error ? e.message : String(e) }),
    );
  }, [client, id]);
  useEffect(() => {
    if (stopped) load();
    return () => {
      asked.current++;
    };
  }, [stopped, load]);
  return { state, load };
}
