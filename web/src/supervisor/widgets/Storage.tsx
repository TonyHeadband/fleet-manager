import { useEffect, useState } from "react";
import type { StorageSummary } from "../../api/generated/StorageSummary";
import type { Task } from "../../api/generated/Task";
import type { Paint } from "../../charts";
import { SegmentedBar } from "../../charts";
import { formatBytes } from "../../shell/counts";
import { look, taskOutcome } from "../../tasks/model";
import { Button } from "../../ui/Button";
import type { WidgetDef, WidgetProps } from "./widgetModel";
import { Widget } from "./Widget";

/** How many tasks the clean-up list shows. */
const LIST_LEN = 5;
/** The bar is read again this often, besides when the tasks change. */
const REFRESH_MS = 60_000;

/** The kinds in a fixed order, so a kind keeps its colour whatever else is on disk. */
const KINDS: { label: string; bytes: (s: StorageSummary) => number; paint: Paint }[] = [
  { label: "worktrees", bytes: (s) => s.worktrees_bytes, paint: { series: 0 } },
  { label: "build output", bytes: (s) => s.build_bytes, paint: { series: 1 } },
  { label: "repo copies", bytes: (s) => s.repos_bytes, paint: { series: 2 } },
  { label: "kept chats", bytes: (s) => s.transcripts_bytes, paint: { series: 3 } },
  { label: "daemon files", bytes: (s) => s.daemon_bytes, paint: { series: 4 } },
];

/** The persona's biggest merged or stopped tasks that are not archived, largest first. */
export function cleanUpList(tasks: Task[], persona: string): Task[] {
  return tasks
    .filter(
      (t) =>
        t.persona === persona &&
        t.kind !== "supervisor" &&
        t.archived_at === null &&
        t.state !== "archived" &&
        (t.state === "stopped" || taskOutcome(t) === "merged"),
    )
    .sort((a, b) => (b.footprint?.total ?? 0) - (a.footprint?.total ?? 0))
    .slice(0, LIST_LEN);
}

const stageOf = (t: Task) => (taskOutcome(t) === "merged" ? "Merged" : look({ kind: "task", task: t }, {})[0]);

function StorageWidget({ persona, tasks, client, onArchive }: WidgetProps) {
  const [summary, setSummary] = useState<StorageSummary | null>(null);
  // A task starting, stopping or being archived moves the numbers: ask again when the tasks' states change.
  const standing = tasks.map((t) => `${t.id}:${t.state}`).join(" ");
  useEffect(() => {
    let live = true;
    const read = () =>
      client.storage().then(
        (s) => live && setSummary(s),
        () => {}, // keep the last numbers; the next read tries again
      );
    read();
    const timer = setInterval(read, REFRESH_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [client, standing]);

  const segments = KINDS.map((k) => ({ label: k.label, value: summary ? k.bytes(summary) : 0, paint: k.paint }));
  const used = segments.reduce((sum, s) => sum + s.value, 0);
  // Scaled to what Fleet Manager holds plus what the disk has left, so the bar shows its share of what is available.
  const max = summary ? used + summary.disk_free_bytes : 0;
  const list = cleanUpList(tasks, persona);

  return (
    <Widget def={STORAGE}>
      <div className="storage-bar">
        <SegmentedBar label="Storage" segments={segments} max={max} format={formatBytes} />
      </div>
      {list.length > 0 && (
        <ul className="storage-list" aria-label="Biggest tasks to clean up">
          {list.map((t) => (
            <li key={t.id} className="storage-row">
              <span className="storage-name">{t.name}</span>
              <span className="storage-stage">{stageOf(t)}</span>
              <span className="storage-size">{t.footprint ? formatBytes(t.footprint.total) : "—"}</span>
              <Button onClick={() => onArchive({ kind: "task", id: t.id, name: t.name })} aria-label={`Archive ${t.name}`}>
                Archive
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Widget>
  );
}

export const STORAGE: WidgetDef = {
  id: "storage",
  title: "Storage",
  minW: 3,
  minH: 1,
  render: (p) => <StorageWidget {...p} />,
};
