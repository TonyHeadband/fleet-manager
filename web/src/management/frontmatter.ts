import type { Effort } from "../api/generated/Effort";
import type { PermissionMode } from "../api/generated/PermissionMode";

export interface DefFields {
  name: string;
  summary: string;
  skills: string[];
  model: string | null;
  mode: PermissionMode | null;
  effort: Effort | null;
}
export interface DefDraft {
  fields: DefFields;
  body: string;
}
export type DefField = "summary" | "skills" | "model" | "mode" | "effort";

// The daemon refuses unknown keys, so a line-based edit that leaves every key the form does not show untouched is
// enough; the YAML subset read is plain or quoted scalars, flow lists and block lists.

const EMPTY: DefFields = { name: "", summary: "", skills: [], model: null, mode: null, effort: null };
const ORDER = ["name", "summary", "skills", "model", "mode", "effort"] as const;

interface Split {
  open: string;
  yaml: string;
  close: string;
  rest: string;
}

// Mirrors the daemon's `split_frontmatter`: the first line is `---` and the next `---` line closes the block.
function split(text: string): Split | null {
  const open = text.startsWith("---\r\n") ? "---\r\n" : text.startsWith("---\n") ? "---\n" : null;
  if (!open) return null;
  const lines = text.slice(open.length).match(/[^\n]*\n|[^\n]+$/g) ?? [];
  let at = open.length;
  for (const line of lines) {
    if (line.trimEnd() === "---") {
      return { open, yaml: text.slice(open.length, at), close: line, rest: text.slice(at + line.length) };
    }
    at += line.length;
  }
  return null;
}

interface Block {
  key: string;
  start: number;
  end: number;
  rest: string;
  block: string[];
}

const KEY = /^([A-Za-z_][\w-]*)[ \t]*:(?:[ \t]+(.*?))?[\r\n]*$/;

function blocks(lines: string[]): Block[] {
  const out: Block[] = [];
  for (let i = 0; i < lines.length; i++) {
    const m = KEY.exec(lines[i] ?? "");
    if (!m) continue;
    let end = i + 1;
    while (end < lines.length && !KEY.test(lines[end] ?? "")) end++;
    // Blank lines and column-0 comments right before the next key belong to the file, not to this key.
    while (end > i + 1 && /^(#.*)?[\r\n]*$/.test(lines[end - 1] ?? "")) end--;
    out.push({ key: m[1] ?? "", start: i, end, rest: m[2] ?? "", block: lines.slice(i + 1, end) });
    i = end - 1;
  }
  return out;
}

function unquote(raw: string): string {
  const s = raw.trim();
  if (s.startsWith('"')) {
    const m = /^"((?:[^"\\]|\\.)*)"/.exec(s);
    if (m) {
      try {
        return JSON.parse(`"${m[1] ?? ""}"`);
      } catch {
        return m[1] ?? "";
      }
    }
  } else if (s.startsWith("'")) {
    const m = /^'((?:[^']|'')*)'/.exec(s);
    if (m) return (m[1] ?? "").replace(/''/g, "'");
  }
  const hash = s.search(/[ \t]#/);
  return (hash >= 0 ? s.slice(0, hash) : s).trim();
}

function splitFlow(inner: string): string[] {
  const items: string[] = [];
  let cur = "";
  let quote = "";
  for (let i = 0; i < inner.length; i++) {
    const c = inner[i] ?? "";
    if (quote) {
      cur += c;
      if (c === "\\" && quote === '"') cur += inner[++i] ?? "";
      else if (c === quote) quote = "";
    } else if (c === '"' || c === "'") {
      quote = c;
      cur += c;
    } else if (c === ",") {
      items.push(cur);
      cur = "";
    } else {
      cur += c;
    }
  }
  items.push(cur);
  return items.map((s) => unquote(s)).filter((s) => s !== "");
}

function scalar(b: Block): string {
  const head = b.rest.trim();
  const all = b.block.map((l) => l.trim());
  const cont = all.filter((l) => l !== "" && !l.startsWith("#"));
  if (/^[|>]/.test(head)) return (head.startsWith("|") ? all : cont).join(head.startsWith("|") ? "\n" : " ");
  if (head === "" || head.startsWith("#")) return "";
  return [unquote(head), ...cont].join(" ").trim();
}

function list(b: Block): string[] {
  const head = b.rest.trim();
  if (head.startsWith("[")) {
    const whole = [head, ...b.block.map((l) => l.trim()).filter((l) => !l.startsWith("#"))].join(" ");
    const close = whole.lastIndexOf("]");
    return splitFlow(whole.slice(1, close < 0 ? undefined : close));
  }
  return b.block
    .map((l) => /^[ \t]*-(?:[ \t]+(.*?))?[\r\n]*$/.exec(l))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => unquote(m[1] ?? ""))
    .filter((s) => s !== "");
}

function fieldsOf(lines: string[]): DefFields {
  const by = new Map<string, Block>();
  for (const b of blocks(lines)) if (!by.has(b.key)) by.set(b.key, b);
  const str = (k: string) => (by.has(k) ? scalar(by.get(k)!) : "");
  const opt = (k: string) => str(k) || null;
  return {
    name: str("name"),
    summary: str("summary"),
    skills: by.has("skills") ? list(by.get("skills")!) : [],
    model: opt("model"),
    mode: opt("mode") as PermissionMode | null,
    effort: opt("effort") as Effort | null,
  };
}

const lineSplit = (yaml: string): string[] => yaml.match(/[^\n]*\n|[^\n]+$/g) ?? [];
const trimBody = (s: string) => s.replace(/^[\r\n]+/, "");

export function parseDef(text: string): DefDraft {
  const s = split(text);
  if (!s) return { fields: { ...EMPTY, skills: [] }, body: text };
  return { fields: fieldsOf(lineSplit(s.yaml)), body: trimBody(s.rest) };
}

const RESERVED = /^(true|false|null|yes|no|on|off|y|n|~|[-+.\d][\d.eE+-]*)$/i;

function quoteIfNeeded(v: string, flow = false): string {
  const plain =
    /^[A-Za-z0-9_/][^\n]*$/.test(v) &&
    !/:([ \t]|$)|[ \t]#|[ \t]$/.test(v) &&
    !RESERVED.test(v) &&
    !/^0[A-Za-z]/.test(v) &&
    !(flow && /[,[\]{}]/.test(v));
  return plain ? v : JSON.stringify(v);
}

function render(key: (typeof ORDER)[number], value: string | string[], eol: string, asBlock: boolean): string[] {
  if (Array.isArray(value)) {
    if (asBlock) return [`${key}:${eol}`, ...value.map((v) => `  - ${quoteIfNeeded(v)}${eol}`)];
    return [`${key}: [${value.map((v) => quoteIfNeeded(v, true)).join(", ")}]${eol}`];
  }
  return [`${key}: ${quoteIfNeeded(value)}${eol}`];
}

const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((v, i) => v === b[i]);

export function writeDef(original: string, draft: DefDraft): string {
  const s = split(original);
  const eol = original.includes("\r\n") ? "\r\n" : "\n";
  if (!s) {
    const lines = ORDER.flatMap((k) => {
      const v = draft.fields[k];
      return v === null || v === "" || (Array.isArray(v) && v.length === 0) ? [] : render(k, v, eol, false);
    });
    if (lines.length === 0 && draft.body === original) return original;
    return `---${eol}${lines.join("")}---${eol}${draft.body}`;
  }

  const lines = lineSplit(s.yaml);
  const have = fieldsOf(lines);
  const bs = blocks(lines);
  const additions: string[] = [];
  // Edits go from the bottom up so earlier line indexes stay valid.
  const edits: { start: number; end: number; with: string[] }[] = [];
  for (const k of ORDER) {
    const next = draft.fields[k];
    const prev = have[k];
    const same = Array.isArray(next) ? sameList(next, prev as string[]) : (next ?? "") === (prev ?? "");
    if (same) continue;
    const b = bs.find((x) => x.key === k);
    const empty = next === null || next === "" || (Array.isArray(next) && next.length === 0);
    if (b) {
      const asBlock = Array.isArray(next) && !b.rest.trim().startsWith("[") && b.block.length > 0;
      const lineEol = /\r?\n$/.exec(lines[b.start] ?? "")?.[0] ?? eol;
      edits.push({ start: b.start, end: b.end, with: empty ? [] : render(k, next, lineEol, asBlock) });
    } else if (!empty) {
      additions.push(...render(k, next, eol, false));
    }
  }
  for (const e of edits.sort((a, b) => b.start - a.start)) lines.splice(e.start, e.end - e.start, ...e.with);
  if (additions.length) {
    const last = lines.length - 1;
    if (last >= 0 && !/\n$/.test(lines[last] ?? "")) lines[last] += eol;
    lines.push(...additions);
  }

  const rest = draft.body === trimBody(s.rest) ? s.rest : draft.body;
  return s.open + lines.join("") + s.close + rest;
}

export function fieldOfError(message: string): DefField | null {
  if (/\bskill `[^`]*` not found/.test(message)) return "skills";
  if (/model must be a name/.test(message)) return "model";
  if (/`summary`/.test(message)) return "summary";
  if (/\bmode \S+ is not allowed/.test(message)) return "mode";
  if (/\beffort \S+ is not allowed/.test(message)) return "effort";
  return null;
}
