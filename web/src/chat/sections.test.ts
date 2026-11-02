import { describe, expect, it } from "vitest";
import type { CallStatus } from "../api/generated/CallStatus";
import type { ChatItem } from "../api/generated/ChatItem";
import { blocks, mergeItems, opensItself, type Section } from "./sections";

const call = (seq: number, status: CallStatus = "done"): ChatItem => ({
  seq,
  at: seq,
  body: { type: "call", id: `c${seq}`, tool: "Read", target: "a.rs", status, result: null, output: null },
});
const reply = (seq: number): ChatItem => ({ seq, at: seq, body: { type: "reply", text: "hi" } });
const sections = (items: ChatItem[]) => blocks(items).filter((b): b is Section => b.kind === "section");

describe("sections", () => {
  it("consecutive_calls_form_one_section", () => {
    const out = blocks([call(1), call(2), call(3)]);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: "section", key: 1 });
    expect((out[0] as Section).calls.map((c) => c.seq)).toEqual([1, 2, 3]);
  });

  it("a_reply_ends_a_section", () => {
    const out = blocks([call(1), reply(2), call(3)]);
    expect(out.map((b) => b.kind)).toEqual(["section", "item", "section"]);
    expect(sections([call(1), reply(2), call(3)]).map((s) => s.key)).toEqual([1, 3]);
  });

  it("a_section_with_a_running_call_opens_itself", () => {
    expect(opensItself(sections([call(1), call(2, "running")])[0]!)).toBe(true);
  });

  it("a_section_with_a_failure_stays_folded", () => {
    expect(opensItself(sections([call(1, "failed"), call(2)])[0]!)).toBe(false);
  });

  it("a_done_section_stays_folded", () => {
    expect(opensItself(sections([call(1), call(2)])[0]!)).toBe(false);
  });

  it("an_updated_call_replaces_its_seq", () => {
    const merged = mergeItems([reply(1), call(2, "running")], [call(2, "done"), reply(3)]);
    expect(merged.map((i) => i.seq)).toEqual([1, 2, 3]);
    expect(merged[1]!.body).toMatchObject({ status: "done" });
  });
});
