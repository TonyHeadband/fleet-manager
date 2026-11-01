import { useMemo, useState } from "react";
import type { Diff } from "../components/PreviewPane";
import { Button } from "../ui/Button";
import { ListRow } from "../ui/ListRow";
import { parseDiff, splitRows, type DiffFile, type DiffLine, type Row } from "./parse";
import "./diff.css";

interface Repo {
  repo: string;
  files: DiffFile[];
}

const keyOf = (repo: string, f: DiffFile) => `${repo}\n${f.path}`;
const dirOf = (path: string) => path.slice(0, path.lastIndexOf("/") + 1);
const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);
/** Folder by folder, root files first: git's own order splits a folder (untracked files come after the rest). */
const byFolder = (a: DiffFile, b: DiffFile) => {
  const [da, db] = [dirOf(a.path), dirOf(b.path)];
  if (da !== db) return da < db ? -1 : 1;
  return a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
};
/** Rows drawn before "Show the other N lines": a regenerated lockfile would otherwise stall the pane. */
export const MAX_ROWS = 2000;

function Counts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="diff-counts">
      {added > 0 && <span data-kind="add">{`+${added}`}</span>}
      {removed > 0 && <span data-kind="del">{`−${removed}`}</span>}
    </span>
  );
}

/**
 * An agent's changes: a tree of the changed files per repo (A, M, D or R, and their line counts) beside the picked
 * file's hunks, side by side by default or in one column. Only the picked file is drawn, so a large diff stays light.
 */
export function DiffView({ diff }: { diff: Diff }) {
  const repos = useMemo<Repo[]>(
    () => (diff.state === "loaded" ? diff.report.workspaces.map((w) => ({ repo: w.repo, files: parseDiff(w.diff, w.truncated).sort(byFolder) })) : []),
    [diff],
  );
  const [picked, setPicked] = useState<string | null>(null);
  const [mode, setMode] = useState<"split" | "unified">("split");
  // The file whose rows are all drawn; picking another draws it in part again.
  const [whole, setWhole] = useState<string | null>(null);

  if (diff.state === "none" || diff.state === "loading") return <p className="note">Loading…</p>;
  if (diff.state === "failed") return <p role="alert" className="diff-failed">{diff.message}</p>;

  const all = repos.flatMap((r) => r.files.map((f) => ({ key: keyOf(r.repo, f), file: f })));
  // A Refresh that drops the picked file falls back to the first one.
  const current = all.find((e) => e.key === picked) ?? all[0];

  return (
    <div className="diff-view" aria-label="Changes">
      <nav className="diff-tree" aria-label="Changed files">
        {repos.map((r) => (
          <div key={r.repo} className="diff-repo-group">
            <div className="diff-repo" data-repo>
              <span className="mono-label">{r.repo}</span>
              <Counts added={r.files.reduce((n, f) => n + f.added, 0)} removed={r.files.reduce((n, f) => n + f.removed, 0)} />
            </div>
            {r.files.length === 0 && <p className="note">No changes</p>}
            {r.files.map((f, i) => {
              const dir = dirOf(f.path);
              const key = keyOf(r.repo, f);
              return (
                <div key={key}>
                  {dir !== "" && (i === 0 || dirOf(r.files[i - 1]!.path) !== dir) && <div className="diff-dir">{dir}</div>}
                  <ListRow name={f.path} selected={key === current?.key} onSelect={() => setPicked(key)}>
                    <span className="diff-mark" data-status={f.status}>
                      {f.status}
                    </span>
                    <span className="diff-name">{nameOf(f.path)}</span>
                    <Counts added={f.added} removed={f.removed} />
                  </ListRow>
                </div>
              );
            })}
          </div>
        ))}
      </nav>
      <section className="diff-file" aria-label="File">
        {current && (
          <>
            <div className="diff-file-head">
              <span className="diff-path">{current.file.path}</span>
              {current.file.oldPath && <span className="diff-from">{`from ${current.file.oldPath}`}</span>}
              <Counts added={current.file.added} removed={current.file.removed} />
              <span className="diff-modes">
                <Button pressed={mode === "split"} onClick={() => setMode("split")}>
                  Split
                </Button>
                <Button pressed={mode === "unified"} onClick={() => setMode("unified")}>
                  Unified
                </Button>
              </span>
            </div>
            <div className="diff-body" data-mode={mode}>
              {current.file.binary && <p className="note">Binary file</p>}
              <Hunks file={current.file} mode={mode} all={whole === current.key} onAll={() => setWhole(current.key)} />
              {current.file.cut && <p className="note">The diff was cut at the size limit.</p>}
            </div>
          </>
        )}
      </section>
    </div>
  );
}

function Cell({ line, side }: { line: DiffLine | null; side: "old" | "new" }) {
  if (!line) return <span className="diff-cell" data-empty />;
  return (
    <span className="diff-cell" data-kind={line.kind}>
      <span className="diff-no">{side === "old" ? line.oldNo : line.newNo}</span>
      <span className="diff-text">{line.text}</span>
    </span>
  );
}

function Hunks({ file, mode, all, onAll }: { file: DiffFile; mode: "split" | "unified"; all: boolean; onAll: () => void }) {
  let budget = all ? Infinity : MAX_ROWS;
  let total = 0;
  const hunks = file.hunks.map((h) => {
    const rows: (Row | DiffLine)[] = mode === "split" ? splitRows(h) : h.lines;
    total += rows.length;
    const shown = rows.slice(0, Math.max(0, budget));
    budget -= shown.length;
    return { header: h.header, rows: shown };
  });
  const hidden = total - hunks.reduce((n, h) => n + h.rows.length, 0);
  return (
    <>
      {hunks
        .filter((h) => h.rows.length > 0)
        .map((h, i) => (
          <div key={i} className="diff-hunk">
            <div className="diff-hunk-head">{h.header}</div>
            {h.rows.map((r, j) =>
              "kind" in r ? (
                <div key={j} className="diff-row" data-row data-kind={r.kind}>
                  <span className="diff-no">{r.oldNo}</span>
                  <span className="diff-no">{r.newNo}</span>
                  <span className="diff-text">{`${r.kind === "add" ? "+" : r.kind === "del" ? "-" : " "}${r.text}`}</span>
                </div>
              ) : (
                <div key={j} className="diff-row" data-row>
                  <Cell line={r.left} side="old" />
                  <Cell line={r.right} side="new" />
                </div>
              ),
            )}
          </div>
        ))}
      {hidden > 0 && (
        <Button className="diff-more" onClick={onAll}>
          {`Show the other ${hidden} lines`}
        </Button>
      )}
    </>
  );
}
