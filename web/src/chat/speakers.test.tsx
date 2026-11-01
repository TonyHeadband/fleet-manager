import { act, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../api/generated/Agent";
import type { ChatBody } from "../api/generated/ChatBody";
import type { ChatItem } from "../api/generated/ChatItem";
import type { Persona } from "../api/generated/Persona";
import { FakeEventSource, agent, factory } from "../test/fixtures";
import { ChatPane, Entry } from "./ChatPane";
import { agentSpeaker, ownerSpeaker, supervisorSpeaker } from "./speakers";

const AT = 86_400 * 19_000 + 14 * 3600 + 9 * 60;
let seq = 0;
const item = (body: ChatBody, at = AT): ChatItem => ({ seq: seq++, at, body });
const prompt = (text: string, at?: number) => item({ type: "prompt", text, from: "owner" }, at);
const reply = (text: string, at?: number) => item({ type: "reply", text }, at);
const call = () => item({ type: "call", id: `t${seq}`, tool: "Read", target: "a.rs", status: "done", result: null, output: null });
const marker = () => item({ type: "marker", kind: "compacted" });

const me = { id: "me", name: "Ada", ready: true, nicknames: { rust: "Ferris" } } as unknown as Persona;
const ferris = (over: Partial<Agent> = {}) => agent({ persona: "me", role: "rust", ...over });

const show = (items: ChatItem[], who: Agent | "supervisor" = ferris()) => {
  const speakers = {
    owner: ownerSpeaker("me", [me]),
    agent: who === "supervisor" ? supervisorSpeaker("me") : agentSpeaker(who, [me]),
  };
  render(<ChatPane baseUrl="" agentId="a1" client={{ chatHistory: vi.fn(), answerAsk: vi.fn() }} factory={factory} speakers={speakers} />);
  act(() => FakeEventSource.for("/chat").emit("page", { items, earlier: 0 }));
};
const speakerLines = () => [...document.querySelectorAll(".chat-speaker")];

beforeEach(() => {
  seq = 0;
  FakeEventSource.reset();
});

describe("a chat names who said each message", () => {
  it("the_owner_is_the_persona_with_its_avatar", () => {
    show([prompt("go")]);
    const own = screen.getByText("go").closest(".chat-prompt")!;
    expect(own.querySelector(".chat-speaker")).toHaveTextContent("Ada");
    expect(own.querySelector(".chat-speaker")).toHaveTextContent("14:09");
    expect(own.querySelector(".avatar")).toHaveAttribute("data-kind", "persona");
    expect(own.querySelector(".avatar")).toHaveAttribute("data-seed", "me");
  });

  it("an_agent_is_named_by_its_nickname_when_the_persona_has_one", () => {
    show([reply("done")]);
    const said = screen.getByText("done").closest(".chat-reply")!;
    expect(said.querySelector(".chat-speaker-name")).toHaveTextContent("Ferris · rust");
    expect(said.querySelector(".avatar")).toHaveAttribute("data-kind", "specialist");
    expect(said.querySelector(".avatar")).toHaveAttribute("data-seed", "rust");
  });

  it("an_agent_without_a_nickname_is_its_plain_name", () => {
    show([reply("done")], ferris({ role: "docs" }));
    expect(document.querySelector(".chat-speaker-name")).toHaveTextContent(/^docs$/);
  });

  it("the_supervisor_is_named_supervisor_with_its_own_avatar", () => {
    show([reply("hello")], "supervisor");
    const said = screen.getByText("hello").closest(".chat-reply")!;
    expect(said.querySelector(".chat-speaker-name")).toHaveTextContent(/^Supervisor$/);
    expect(said.querySelector(".avatar")).toHaveAttribute("data-kind", "specialist");
    expect(said.querySelector(".avatar")).toHaveAttribute("data-seed", "supervisor:me");
  });
});

describe("a reply with no speaker", () => {
  it("still_shows_its_text", () => {
    render(<Entry item={{ seq: 1, at: 0, body: { type: "reply", text: "still here" } }} />);
    expect(screen.getByText("still here")).toBeInTheDocument();
  });
});

describe("consecutive messages from one speaker", () => {
  it("share_one_header_within_five_minutes", () => {
    show([reply("one"), reply("two", AT + 299)]);
    expect(speakerLines()).toHaveLength(1);
    const second = screen.getByText("two").closest(".chat-reply")!;
    expect(second.querySelector(".avatar")).toBeNull();
    expect(second).toHaveAttribute("data-continued");
  });

  it("show_the_header_again_after_five_minutes", () => {
    show([reply("one"), reply("two", AT + 301)]);
    expect(speakerLines()).toHaveLength(2);
  });

  it("show_the_header_again_after_a_tool_call_or_a_marker", () => {
    show([reply("one"), call(), reply("two"), marker(), reply("three")]);
    expect(speakerLines()).toHaveLength(3);
  });

  it("show_the_header_again_when_the_speaker_changes", () => {
    show([prompt("a"), reply("b"), prompt("c")]);
    expect(speakerLines()).toHaveLength(3);
  });

  it("keep_their_text_aligned_under_the_first_ones", () => {
    show([reply("one"), reply("two")]);
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-msg-body \{[^}]*grid-column: 2;/);
  });
});

describe("rows that are not messages", () => {
  it("tool_calls_and_markers_have_no_header_or_avatar", () => {
    show([call(), marker()]);
    expect(speakerLines()).toHaveLength(0);
    expect(document.querySelector(".avatar")).toBeNull();
  });
});

describe("chat.css speaker rules", () => {
  const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
  const rules = [...css.matchAll(/(\.chat-msg[^{]*|\.chat-speaker-time[^{]*) \{([^}]*)\}/g)];

  it("are_there_and_use_tokens_only", () => {
    expect(rules.length).toBeGreaterThan(2);
    for (const [, , body] of rules) {
      expect(body!.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
      expect(body).not.toMatch(/#[0-9a-fA-F]{3,8}\b|gradient|border-radius/);
    }
  });

  it("size_the_avatar_in_the_portrait_ratio_from_tokens", () => {
    expect(css).toMatch(/\.chat-msg > \.avatar \{[^}]*width: calc\(var\(--size-meta\) \* \d+ \* 3 \/ 4\); height: calc\(var\(--size-meta\) \* \d+\);/);
  });

  it("set_mono_with_the_mono_weight", () => {
    for (const [, sel, body] of rules) if (body!.includes("font-family: var(--font-mono)")) expect(body, sel).toContain("font-weight: var(--weight-mono)");
  });
});
