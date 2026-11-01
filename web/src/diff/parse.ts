/** A repo's unified diff (`git diff` text, as the daemon sends it) read as files, hunks and numbered lines. */

export type FileStatus = "A" | "M" | "D" | "R";
export interface DiffLine {
  kind: "ctx" | "add" | "del";
  text: string;
  oldNo: number | null;
  newNo: number | null;
}
export interface Hunk {
  header: string;
  lines: DiffLine[];
}
export interface DiffFile {
  path: string;
  oldPath: string | null;
  status: FileStatus;
  binary: boolean;
  added: number;
  removed: number;
  hunks: Hunk[];
  /** The daemon's size cap cut the diff inside this file. */
  cut: boolean;
}
export interface Row {
  left: DiffLine | null;
  right: DiffLine | null;
}

const DEV_NULL = "/dev/null";

const ESCAPES: Record<string, number> = { a: 7, b: 8, t: 9, n: 10, v: 11, f: 12, r: 13, '"': 34, "\\": 92 };

/**
 * A path as git writes it: quoted C-style when it holds unusual bytes (`core.quotePath`, on by default, writes
 * non-ASCII as octal escapes of its UTF-8 bytes). An escape git never writes is kept as its character, so a bad path
 * still shows rather than failing the whole view.
 */
export function unquote(raw: string): string {
  if (!(raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"'))) return raw;
  const bytes: number[] = [];
  const body = raw.slice(1, -1);
  for (let i = 0; i < body.length; i++) {
    const c = body[i]!;
    if (c !== "\\" || i === body.length - 1) {
      bytes.push(...new TextEncoder().encode(c));
      continue;
    }
    const next = body[++i]!;
    const octal = /^[0-7]{3}/.exec(body.slice(i));
    if (octal) {
      bytes.push(parseInt(octal[0], 8));
      i += 2;
    } else bytes.push(...(next in ESCAPES ? [ESCAPES[next]!] : new TextEncoder().encode(next)));
  }
  return new TextDecoder("utf-8").decode(new Uint8Array(bytes));
}

/** A `---`/`+++` path: git ends an unquoted one with a tab when it holds a space, and prefixes it with `a/` or `b/`. */
function headerPath(raw: string): string {
  const p = unquote(raw.replace(/\t$/, "").trim());
  return p === DEV_NULL ? DEV_NULL : p.replace(/^[ab]\//, "");
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export function parseDiff(text: string, truncated: boolean): DiffFile[] {
  const files: DiffFile[] = [];
  let file: (DiffFile & { from: string | null; to: string | null; header: string; mode: "new" | "deleted" | null }) | null = null;
  let hunk: Hunk | null = null;
  let oldNo = 0;
  let newNo = 0;

  const close = () => {
    if (!file) return;
    const { from, to, header, mode, ...f } = file;
    // Paths come from `---`/`+++`, a rename or a binary line; the `diff --git` line is ambiguous with spaces, so it
    // is the last resort, for a change with none of those (a mode change, an empty file added or deleted).
    const fallback = header.match(/^diff --git a\/(.+) b\/(.+)$/);
    const before = mode === "new" ? DEV_NULL : (from ?? fallback?.[1] ?? DEV_NULL);
    const after = mode === "deleted" ? DEV_NULL : (to ?? fallback?.[2] ?? DEV_NULL);
    if (before === DEV_NULL) f.status = "A";
    else if (after === DEV_NULL) f.status = "D";
    else if (before !== after) f.status = "R";
    f.path = after === DEV_NULL ? before : after;
    f.oldPath = f.status === "R" ? before : null;
    files.push(f);
  };

  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      close();
      file = { path: "", oldPath: null, status: "M", binary: false, added: 0, removed: 0, hunks: [], cut: false, from: null, to: null, header: line, mode: null };
      hunk = null;
      continue;
    }
    if (!file) continue;
    const h = HUNK.exec(line);
    if (h) {
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      oldNo = Number(h[1]);
      newNo = Number(h[2]);
      continue;
    }
    if (hunk) {
      if (line.startsWith("+")) {
        hunk.lines.push({ kind: "add", text: line.slice(1), oldNo: null, newNo: newNo++ });
        file.added++;
      } else if (line.startsWith("-")) {
        hunk.lines.push({ kind: "del", text: line.slice(1), oldNo: oldNo++, newNo: null });
        file.removed++;
      } else if (line.startsWith(" ")) {
        hunk.lines.push({ kind: "ctx", text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
      }
      // `\ No newline at end of file`, and the empty last line of the text, add nothing.
      continue;
    }
    if (line.startsWith("--- ")) file.from = headerPath(line.slice(4));
    else if (line.startsWith("+++ ")) file.to = headerPath(line.slice(4));
    else if (line.startsWith("rename from ")) file.from = unquote(line.slice("rename from ".length));
    else if (line.startsWith("rename to ")) file.to = unquote(line.slice("rename to ".length));
    else if (line.startsWith("new file mode ")) file.mode = "new";
    else if (line.startsWith("deleted file mode ")) file.mode = "deleted";
    else if (line.startsWith("Binary files ")) {
      file.binary = true;
      const b = line.match(/^Binary files (.+) and (.+) differ$/);
      if (b) {
        file.from = headerPath(b[1]!);
        file.to = headerPath(b[2]!);
      }
    }
  }
  close();
  if (truncated && files.length > 0) files[files.length - 1]!.cut = true;
  return files;
}

/** Side by side: each run of removed lines faces the run of added lines after it, line for line. */
export function splitRows(hunk: Hunk): Row[] {
  const rows: Row[] = [];
  let dels: DiffLine[] = [];
  let adds: DiffLine[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(dels.length, adds.length); i++) rows.push({ left: dels[i] ?? null, right: adds[i] ?? null });
    dels = [];
    adds = [];
  };
  for (const l of hunk.lines) {
    if (l.kind === "del") {
      if (adds.length > 0) flush();
      dels.push(l);
    } else if (l.kind === "add") adds.push(l);
    else {
      flush();
      rows.push({ left: l, right: l });
    }
  }
  flush();
  return rows;
}
