import { useMemo, useState } from "react";
import type { Diff } from "../components/PreviewPane";
import { Button } from "../ui/Button";
import { ListRow } from "../ui/ListRow";
import { parseDiff, splitRows, type DiffFile, type DiffLine, type Hunk } from "./parse";
import "./diff.css";

interface Repo {
  repo: string;
  files: DiffFile[];
}

const keyOf = (repo: string, f: DiffFile) => `${repo}\n${f.path}`;
const dirOf = (path: string) => path.slice(0, path.lastIndexOf("/") + 1);
const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);

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
    () => (diff.state === "loaded" ? diff.report.workspaces.map((w) => ({ repo: w.repo, files: parseDiff(w.diff, w.truncated) })) : []),
    [diff],
  );
  const [picked, setPicked] = useState<string | null>(null);
  const [mode, setMode] = useState<"split" | "unified">("split");

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
              {current.file.hunks.map((h, i) => (
                <HunkView key={i} hunk={h} mode={mode} />
              ))}
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

function HunkView({ hunk, mode }: { hunk: Hunk; mode: "split" | "unified" }) {
  return (
    <div className="diff-hunk">
      <div className="diff-hunk-head">{hunk.header}</div>
      {mode === "split"
        ? splitRows(hunk).map((r, i) => (
            <div key={i} className="diff-row" data-row>
              <Cell line={r.left} side="old" />
              <Cell line={r.right} side="new" />
            </div>
          ))
        : hunk.lines.map((l, i) => (
            <div key={i} className="diff-row" data-row data-kind={l.kind}>
              <span className="diff-no">{l.oldNo}</span>
              <span className="diff-no">{l.newNo}</span>
              <span className="diff-text">{`${l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "}${l.text}`}</span>
            </div>
          ))}
    </div>
  );
}
