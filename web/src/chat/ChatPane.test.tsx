import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FleetError } from "../api/client";
import type { ChatAsk } from "../api/generated/ChatAsk";
import type { ChatBody } from "../api/generated/ChatBody";
import type { ChatItem } from "../api/generated/ChatItem";
import type { TaskEventKind } from "../api/generated/TaskEventKind";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, agent, factory, fakeClient, persona, product, schema } from "../test/fixtures";
import { ChatPane, type ChatClient } from "./ChatPane";

// 14:09 UTC, as the footer's clock reads it.
const AT = 86_400 * 19_000 + 14 * 3600 + 9 * 60;
let seq = 0;
const item = (body: ChatBody): ChatItem => ({ seq: seq++, at: AT, body });
const prompt = (text: string, from: "owner" | "system" = "owner") => item({ type: "prompt", text, from });
const reply = (text: string) => item({ type: "reply", text });
const call = (tool: string, target: string, status: "running" | "done" | "failed" = "done", result: string | null = null, output: string | null = null) =>
  item({ type: "call", id: `t${seq}`, tool, target, status, result, output });
const marker = (kind: "new_session" | "compacted" | "session_ended") => item({ type: "marker", kind });

const chat = () => FakeEventSource.for("/chat");
const page = (items: ChatItem[], earlier = 0) => act(() => chat().emit("page", { items, earlier }));
const more = (items: ChatItem[]) => act(() => chat().emit("items", items));
const asks = (a: ChatAsk | null) => act(() => chat().emit("ask", a));
const fakeChat = (over: Partial<ChatClient> = {}): ChatClient => ({
  chatHistory: vi.fn().mockResolvedValue({ items: [], earlier: 0 }),
  answerAsk: vi.fn().mockResolvedValue(undefined),
  ...over,
});
const start = (client = fakeChat()) => {
  render(<ChatPane baseUrl="" agentId="a1" client={client} factory={factory} />);
  return client;
};
const feed = () => screen.getByRole("log", { name: "Chat" });
/** jsdom lays nothing out: the feed's heights are what the test says. */
const sized = (el: HTMLElement, h: { scroll: number; client: number }) => {
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => h.scroll });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => h.client });
};

const permission = (id = "k1"): ChatAsk => ({
  id,
  kind: {
    type: "permission",
    tool: "Bash",
    input: { command: "git push" },
    options: [
      { id: "allow", label: "Allow once", description: null },
      { id: "allow_session", label: "Allow for this session", description: null },
      { id: "deny", label: "Deny", description: null },
    ],
  },
  answered: null,
});

beforeEach(() => {
  seq = 0;
  FakeEventSource.reset();
});

describe("ChatPane", () => {
  it("owner_prompts_sit_right_and_replies_render_markdown", () => {
    start();
    page([
      prompt("Write the failing test for `backfill_tasks`"),
      reply("Some **b** and `c`:\n\n- one\n- two\n\n```\nlet x = 1;\n```\n\n<script>alert(1)</script> and [docs](https://example.test)."),
    ]);
    const own = screen.getByText("backfill_tasks").closest(".chat-prompt")!;
    expect(own).toHaveAttribute("data-from", "owner");
    const answer = feed().querySelector(".chat-reply")!;
    expect(answer.querySelector("strong")).toHaveTextContent("b");
    expect(answer.querySelector("p > code")).toHaveTextContent("c");
    expect(answer.querySelectorAll("li")).toHaveLength(2);
    expect(answer.querySelector("pre code")).toHaveTextContent("let x = 1;");
    expect(answer.querySelector("script"), "raw HTML stays text").toBeNull();
    expect(answer).toHaveTextContent("<script>alert(1)</script>");
    const link = within(answer as HTMLElement).getByRole("link", { name: "docs" });
    expect(link).toHaveAttribute("href", "https://example.test");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-prompt\[data-from="owner"\] \{[^}]*align-self: flex-end;[^}]*background: var\(--raised\);/);
  });

  it("system_prompts_and_markers_are_lines", () => {
    start();
    page([prompt("Resume the task", "system"), marker("compacted"), marker("new_session")]);
    expect(screen.getByText("Resume the task")).toHaveClass("chat-line");
    expect(feed().querySelector(".chat-prompt")).toBeNull();
    expect(screen.getByText("Context compacted")).toHaveClass("chat-line");
    expect(screen.getByText("New session")).toHaveClass("chat-line");
  });

  it("event_rows_render_as_system_lines_with_their_tone", () => {
    start();
    const event = (kind: TaskEventKind, text: string) =>
      item({ type: "event", task: "t1", task_title: "Build X", kind, text });
    page([
      prompt("Resume the task", "system"),
      event("needs_you", "needs you · approve the plan"),
      event("checks_failed", "checks failed"),
      event("checks_passed", "checks passed"),
      event("pr_opened", "PR opened"),
      event("pr_merged", "PR merged"),
      event("room_message", "Build X in Untitled: PR is up"),
      event("room_ask", "Build X in Untitled: can I change the schema?"),
    ]);
    const rows = [...feed().querySelectorAll(".chat-line[data-event]")];
    expect(rows.map((r) => r.getAttribute("data-from"))).toEqual(Array(7).fill("system"));
    expect(rows[0]!.querySelector(".chat-event-title")).toHaveTextContent("Build X");
    expect(rows[0]).toHaveTextContent("Build X needs you · approve the plan");
    expect(rows.map((r) => r.querySelector(".chat-mark")!.getAttribute("data-tone"))).toEqual(["accent", "danger", "done", "done", "done", "done", "accent"]);
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-event-title \{[^}]*color: var\(--text-high\);/);
    expect(css).toMatch(/\.chat-mark\[data-tone="accent"\] \{[^}]*background: var\(--fill\);/);
    expect(css).toMatch(/\.chat-mark\[data-tone="danger"\] \{[^}]*background: var\(--danger\);/);
  });

  it("sections_fold_and_open", async () => {
    start();
    page([
      reply("look"),
      call("Read", "src/a.rs", "done", "12 lines"),
      call("Grep", "fn backfill", "done", "3 files"),
      reply("then"),
      call("Bash", "cargo test", "failed", "1 failed", "left: 1  right: 0"),
      reply("now"),
      call("Bash", "git diff --stat", "running"),
    ]);
    const done = screen.getByRole("button", { name: "2 calls · Read, Grep" });
    expect(done).toHaveAttribute("aria-expanded", "false");
    expect(done.querySelector(".plus-minus")).not.toHaveAttribute("data-open");
    expect([...done.querySelectorAll(".chat-mark")].map((m) => m.getAttribute("data-status"))).toEqual(["done", "done"]);
    expect(screen.queryByText("src/a.rs")).toBeNull();
    await userEvent.click(done);
    expect(done).toHaveAttribute("aria-expanded", "true");
    const read = screen.getByText("src/a.rs").closest(".chat-call")!;
    expect(read).toHaveTextContent("Read");
    expect(read).toHaveTextContent("12 lines");
    expect(screen.getByText("fn backfill").closest(".chat-call")).toHaveTextContent("3 files");
    // A failure opens itself, its output under it.
    const failed = screen.getByText("cargo test").closest(".chat-section") as HTMLElement;
    expect(within(failed).getByRole("button", { name: "1 call · Bash" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("cargo test").closest(".chat-call")).toHaveTextContent("1 failed");
    expect(screen.getByText(/left: 1/)).toHaveClass("chat-call-output");
    // The running section is open, its + a −.
    const running = screen.getByText("git diff --stat").closest(".chat-section")!;
    const head = within(running as HTMLElement).getByRole("button", { name: "1 call · Bash" });
    expect(head).toHaveAttribute("aria-expanded", "true");
    expect(head.querySelector(".plus-minus")).toHaveAttribute("data-open");
    expect(screen.getByText("git diff --stat").closest(".chat-call")).toHaveTextContent("running");
    // The owner can fold it.
    await userEvent.click(head);
    expect(screen.queryByText("git diff --stat")).toBeNull();
  });

  it("opens_at_the_latest_and_keeps_to_the_bottom", () => {
    start();
    const h = { scroll: 1000, client: 200 };
    sized(feed(), h);
    page([reply("first"), reply("second")]);
    expect(feed().scrollTop).toBe(1000);
    h.scroll = 1400;
    more([reply("third")]);
    expect(feed().scrollTop).toBe(1400);
  });

  it("does_not_move_a_reader_who_scrolled_up", () => {
    start();
    const h = { scroll: 1000, client: 200 };
    sized(feed(), h);
    page([reply("first"), reply("second")]);
    feed().scrollTop = 300;
    fireEvent.scroll(feed());
    h.scroll = 1400;
    more([reply("third")]);
    expect(feed().scrollTop).toBe(300);
  });

  it("loads_earlier_sections_on_scrolling_up", async () => {
    seq = 0;
    const older = [reply("the oldest"), call("Read", "a.rs")];
    const client = start(fakeChat({ chatHistory: vi.fn().mockResolvedValue({ items: older, earlier: 0 }) }));
    // The earlier page, once in, adds 600 to the feed's height.
    Object.defineProperty(feed(), "scrollHeight", { configurable: true, get: () => (feed().textContent!.includes("the oldest") ? 1600 : 1000) });
    Object.defineProperty(feed(), "clientHeight", { configurable: true, get: () => 200 });
    page([reply("newer"), reply("newest")], 2);
    expect(screen.getByRole("button", { name: "↑ 2 earlier sections" })).toBeInTheDocument();
    feed().scrollTop = 0;
    await act(async () => fireEvent.scroll(feed()));
    expect(client.chatHistory).toHaveBeenCalledWith("a1", 2, 200);
    expect(screen.getByText("the oldest")).toBeInTheDocument();
    expect(screen.queryByText(/earlier section/)).toBeNull();
    expect(feed().scrollTop, "the reader stays on what they were reading").toBe(600);
  });

  it("a_permission_ask_answers_with_its_buttons", async () => {
    const client = start();
    page([reply("pushing")]);
    asks(permission());
    const ask = screen.getByRole("group", { name: "Wants to run" });
    expect(ask).toHaveTextContent("Bash");
    expect(ask).toHaveTextContent("git push");
    expect(within(ask).getAllByRole("button").map((b) => b.textContent)).toEqual(["Allow once", "Allow for this session", "Deny"]);
    await userEvent.click(within(ask).getByRole("button", { name: "Allow once" }));
    expect(client.answerAsk).toHaveBeenCalledWith("a1", "k1", { choice: "allow", text: null });
    expect(ask).toHaveTextContent("Answered · Allow once");
    expect(within(ask).queryAllByRole("button")).toHaveLength(0);
    // The daemon clears the ask: the answered message stays where it was.
    asks(null);
    expect(screen.getByRole("group", { name: "Wants to run" })).toHaveTextContent("Answered · Allow once");
  });

  it("an_ask_answered_elsewhere_shows_it_answered", async () => {
    start(fakeChat({ answerAsk: vi.fn().mockRejectedValue(new FleetError("already answered", undefined, undefined, 409)) }));
    page([]);
    asks(permission());
    const ask = screen.getByRole("group", { name: "Wants to run" });
    await userEvent.click(within(ask).getByRole("button", { name: "Deny" }));
    expect(ask).toHaveTextContent("Answered");
    expect(within(ask).queryAllByRole("button")).toHaveLength(0);
  });

  it("a_question_ask_answers_with_its_options", async () => {
    const client = start();
    page([]);
    asks({
      id: "q1",
      kind: {
        type: "question",
        questions: [
          {
            header: "Store",
            question: "Which database?",
            options: [
              { id: "0.0", label: "Postgres", description: "the one we run" },
              { id: "0.1", label: "SQLite", description: null },
            ],
            multi_select: false,
          },
        ],
      },
      answered: null,
    });
    const ask = screen.getByRole("group", { name: "Asks" });
    expect(ask).toHaveTextContent("Which database?");
    expect(within(ask).getByText("the one we run")).toHaveClass("chat-option-description");
    await userEvent.click(within(ask).getByRole("button", { name: /SQLite/ }));
    expect(client.answerAsk).toHaveBeenCalledWith("a1", "q1", { choice: "0.1", text: null });
    expect(ask).toHaveTextContent("Answered · SQLite");
  });

  it("an_option_press_never_sends_the_typed_answer", async () => {
    const client = start();
    page([]);
    const question = (id: string): ChatAsk => ({
      id,
      kind: {
        type: "question",
        questions: [
          {
            header: "",
            question: "Which database?",
            options: [
              { id: "0.0", label: "Postgres", description: null },
              { id: "0.1", label: "SQLite", description: null },
            ],
            multi_select: false,
          },
        ],
      },
      answered: null,
    });
    asks(question("q1"));
    let ask = screen.getByRole("group", { name: "Asks" });
    await userEvent.type(within(ask).getByRole("textbox", { name: "Another answer" }), "a flat file");
    await userEvent.click(within(ask).getByRole("button", { name: "SQLite" }));
    expect(client.answerAsk).toHaveBeenLastCalledWith("a1", "q1", { choice: "0.1", text: null });
    expect(ask).toHaveTextContent("Answered · SQLite");
    // The typed answer goes only by its own submit: Enter in its field.
    asks(question("q2"));
    ask = screen.getAllByRole("group", { name: "Asks" }).at(-1)!;
    await userEvent.type(within(ask).getByRole("textbox", { name: "Another answer" }), "a flat file{Enter}");
    expect(client.answerAsk).toHaveBeenLastCalledWith("a1", "q2", { choice: "", text: "a flat file" });
    expect(ask).toHaveTextContent("Answered · a flat file");
  });

  it("a_screen_ask_shows_the_screen", () => {
    start();
    page([]);
    asks({ id: "s1", kind: { type: "screen", text: "Do you trust the files in this folder?\n❯ 1. Yes   \n\n\n" }, answered: null });
    const ask = screen.getByRole("group", { name: "Waiting on its screen" });
    expect(ask.querySelector("pre")!.textContent).toBe("Do you trust the files in this folder?\n❯ 1. Yes");
    expect(within(ask).queryAllByRole("button")).toHaveLength(0);
    expect(ask).toHaveTextContent("Answer it in the reply box");
  });

  it("an_empty_chat_says_so", () => {
    start();
    expect(feed(), "nothing said before the page comes").toBeEmptyDOMElement();
    page([]);
    expect(screen.getByText("Nothing yet: the chat starts with the agent's first message.")).toHaveClass("chat-empty");
    more([reply("hello")]);
    expect(screen.queryByText(/Nothing yet/)).toBeNull();
  });

  it("an_empty_chat_with_an_ask_or_an_end_says_nothing_more", () => {
    start();
    page([]);
    asks(permission());
    expect(screen.queryByText(/Nothing yet/)).toBeNull();
    asks(null);
    act(() => chat().emit("ended", AT));
    expect(screen.queryByText(/Nothing yet/)).toBeNull();
  });

  it("a_session_end_clears_waiting_prompts", async () => {
    start();
    page([reply("pushing")]);
    asks(permission("k1"));
    await userEvent.click(within(screen.getByRole("group", { name: "Wants to run" })).getByRole("button", { name: "Allow once" }));
    asks(permission("k2"));
    expect(screen.getAllByRole("group", { name: "Wants to run" })).toHaveLength(2);
    act(() => chat().emit("ended", AT));
    const left = screen.getAllByRole("group", { name: "Wants to run" });
    expect(left).toHaveLength(1);
    expect(left[0]).toHaveTextContent("Answered · Allow once");
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();
  });

  it("the_permission_buttons_stay_on_one_row", () => {
    start();
    page([]);
    asks(permission());
    const row = within(screen.getByRole("group", { name: "Wants to run" })).getByRole("button", { name: "Deny" }).parentElement!;
    expect(row).toHaveAttribute("data-row");
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-ask-options\[data-row\] \{[^}]*flex-wrap: nowrap;/);
    expect(css).toMatch(/\.chat-ask-options\[data-row\] button \{[^}]*white-space: nowrap;[^}]*text-overflow: ellipsis;/);
    // The default control padding leaves the three labels a few px short in the 420px panel.
    expect(css).toMatch(/\.chat-ask-options\[data-row\] button \{[^}]*padding-inline: var\(--layout-badge-x\);/);
  });

  it("chat_css_uses_tokens_only", () => {
    const css = readFileSync("src/chat/chat.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/aspect-ratio|border-radius|gradient/);
    // The section's open motion is motion.css's, under the motion guard.
    const motion = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    expect(motion).toMatch(/@media \(prefers-reduced-motion: no-preference\) \{[^@]*\.chat-calls \{ animation: fui-menu var\(--motion-menu\) var\(--ease-push\) both; \}/);
    expect(css).not.toMatch(/animation|transition/);
  });
});

describe("the Chat in the app", () => {
  const ready = {
    personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }),
    products: vi.fn().mockResolvedValue({ products: [product("p1", "Pie")] }),
  };
  const snapshot = (...agents: ReturnType<typeof agent>[]) => act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
  const panel = () => screen.getByRole("complementary", { name: "Detail" });
  afterEach(() => {
    delete document.documentElement.dataset.animations;
  });

  it("a_stopped_agent_ends_with_session_ended_and_no_reply_box", async () => {
    // Even a schema that offered Answer to a stopped agent gets no reply box: the chat's last line closes the view.
    const loose = structuredClone(schema) as UiSchema;
    loose.actions = loose.actions.map((a) => (a.input !== null ? { ...a, when: { type: "always" } } : a));
    render(<App client={fakeClient({ ...ready, uiSchema: vi.fn().mockResolvedValue(loose) })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    snapshot(agent({ id: "a", status: "stopped" }));
    await userEvent.click(screen.getByText("fix the tests"));
    page([reply("Committed."), marker("session_ended")]);
    act(() => chat().emit("ended", AT));
    const log = within(panel()).getByRole("log", { name: "Chat" });
    expect(log.lastElementChild).toHaveTextContent("Session ended · 14:09");
    expect(within(log).getAllByText(/Session ended/)).toHaveLength(1);
    expect(within(panel()).queryByRole("textbox")).toBeNull();
  });

  it("the_reply_box_takes_the_focus_only_when_the_agent_needs_you", async () => {
    render(<App client={fakeClient(ready)} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    snapshot(agent({ id: "a" }), agent({ id: "b", goal: "pie job", status: "review" }), agent({ id: "c", goal: "tart job", status: "needs_you", blocked_reason: "question" }));
    await userEvent.click(screen.getByText("fix the tests"));
    expect(within(panel()).getByPlaceholderText("Answer…")).not.toHaveFocus();
    await userEvent.click(screen.getByText("pie job"));
    expect(within(panel()).getByPlaceholderText("Answer…")).not.toHaveFocus();
    await userEvent.click(screen.getByText("tart job"));
    expect(within(panel()).getByPlaceholderText("Answer…")).toHaveFocus();
  });

  it("the_chat_does_not_reflow_while_the_panel_pushes_in", async () => {
    render(<App client={fakeClient(ready)} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(within(screen.getByRole("main")).getByText("pie job"));
    expect(panel()).toHaveAttribute("data-phase", "opening");
    // The chat sits in one of the panel's own children, which motion.css holds at the panel's final width.
    const root = panel().querySelector(".chat")!;
    const child = [...panel().children].find((c) => c.contains(root));
    expect(child, "the chat is inside a child of the panel").toBeTruthy();
    const motion = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    expect(motion).toMatch(/\.detail\[data-phase="opening"\] > \*, \.detail\[data-phase="closing"\] > \* \{[^}]*min-width: calc\(var\(--panel-final\) - 2px\);/);
    // Nothing in the chat sizes itself off the window, which moves on its own as the panel pushes.
    const css = readFileSync("src/chat/chat.css", "utf8");
    expect(css).not.toMatch(/\d+(vw|vh|vmin|vmax)\b|position: fixed/);
  });
});
