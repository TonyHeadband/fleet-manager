import type { ChatItem } from "../api/generated/ChatItem";

export type Section = { kind: "section"; key: number; calls: ChatItem[] };
export type Block = { kind: "item"; item: ChatItem } | Section;

// Mirrors the daemon's "earlier sections" count: a run of consecutive calls is one block, any other item is one.
export function blocks(items: ChatItem[]): Block[] {
  const out: Block[] = [];
  for (const item of items) {
    if (item.body.type !== "call") {
      out.push({ kind: "item", item });
      continue;
    }
    const last = out[out.length - 1];
    if (last?.kind === "section") last.calls.push(item);
    else out.push({ kind: "section", key: item.seq, calls: [item] });
  }
  return out;
}

export function opensItself(section: Section): boolean {
  return section.calls.some(
    (c) => c.body.type === "call" && (c.body.status === "running" || c.body.status === "failed"),
  );
}

// An updated item arrives again with its seq; the newer one wins.
export function mergeItems(prev: ChatItem[], next: ChatItem[]): ChatItem[] {
  const bySeq = new Map<number, ChatItem>();
  for (const item of [...prev, ...next]) bySeq.set(item.seq, item);
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}
