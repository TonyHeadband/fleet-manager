import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FleetError } from "../api/client";
import type { ChatAsk } from "../api/generated/ChatAsk";
import type { ChatBody } from "../api/generated/ChatBody";
import type { ChatItem } from "../api/generated/ChatItem";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, agent, factory, fakeClient, persona, product, schema } from "../test/fixtures";
import { ChatPane, Entry, type ChatClient } from "./ChatPane";
import type { ChatSpeakers } from "./speakers";

const speakers: ChatSpeakers = { owner: { id: "owner", name: "Ada" }, agent: { id: "a1", name: "docs" } };

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
  render(<ChatPane baseUrl="" agentId="a1" client={client} factory={factory} speakers={speakers} />);
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
  it("owner_prompts_and_replies_render_markdown", () => {
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
  });

  it("system_prompts_and_markers_are_lines", () => {
    start();
    page([prompt("Resume the task", "system"), marker("compacted"), marker("new_session")]);
    expect(screen.getByText("Resume the task")).toHaveClass("chat-line");
    expect(feed().querySelector(".chat-prompt")).toBeNull();
    expect(screen.getByText("Context compacted")).toHaveClass("chat-line");
    expect(screen.getByText("New session")).toHaveClass("chat-line");
  });

  it("a_task_event_body_draws_no_row", () => {
    start();
    // The daemon no longer sends event bodies; one that still arrives is not drawn as a notice row.
    page([prompt("Resume the task", "system"), item({ type: "event", task: "t1", task_title: "Build X", kind: "checks_passed", text: "checks passed" } as unknown as ChatBody), reply("done")]);
    expect(screen.getByText("Resume the task")).toBeInTheDocument();
    expect(screen.getByText("done")).toBeInTheDocument();
    expect(feed().querySelector("[data-event]")).toBeNull();
    expect(feed()).not.toHaveTextContent("Build X");
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

  it("an_entry_is_a_message_only_with_a_speaker_and_other_rows_stay_narrow", () => {
    const html = (i: ChatItem) => {
      const { container, unmount } = render(<Entry item={i} speaker={speakers.agent} />);
      const out = container.innerHTML;
      unmount();
      return out;
    };
    // A system prompt is a plain line, folded like any message (it wraps its fold box, which a short one never shows).
    expect(html(prompt("Resumed", "system"))).toContain('<p class="chat-line" data-from="system">Resumed</p>');
    expect(html(marker("compacted"))).toBe('<p class="chat-line" data-marker="compacted">Context compacted</p>');
    const named = render(<Entry item={reply("Done.")} speaker={{ id: "x", name: "Fix login" }} />);
    expect(named.container.querySelector(".chat-reply .chat-speaker")).toHaveTextContent("Fix login");
  });

  it("chat_css_uses_tokens_only", () => {
    const css = readFileSync("src/chat/chat.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    // The call target's width and every line-height are tokens too: `1px` is the only literal.
    expect(css).not.toMatch(/12ch\b/);
    expect(css).not.toMatch(/line-height:\s*[\d.]/);
    expect(css).toMatch(/\.chat-call-target \{[^}]*min-width: var\(--layout-call-target-min\);/);
    expect(css).toMatch(/\.chat-line\[data-from="system"\] \{[^}]*line-height: var\(--layout-chat-line\);/);
    expect(css).toMatch(/\.markdown \{ line-height: var\(--layout-chat-line\); \}/);
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
  it("a_long_failed_call_line_wraps_inside_the_chat", () => {
    const long = 'set title failed: this task already has a title: Definitions say "No active persona" instead of loading forever (#478)';
    start();
    page([call("Bash", "set_title", "failed", long, `${"x".repeat(300)}\n${long}`), reply("y".repeat(300)), reply("| a | b |\n|---|---|\n| 1 | 2 |")]);
    expect(document.querySelector(".markdown table")).toHaveTextContent("a b 1 2");
    const result = screen.getByText(long, { selector: ".chat-call-result" });
    expect(result).toBeInTheDocument();
    expect(document.querySelector(".chat-call-output")).toHaveTextContent("x".repeat(300));
    const css = readFileSync("src/chat/chat.css", "utf8");
    const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.replace(/[.[\]="]/g, "\\$&")} \\{([^}]*)\\}`))?.[1] ?? "";
    // The tool and its target share a line however long the target is: it counts only its minimum (basis 0) when the
    // row's lines are built, then grows into the room and is cut with an ellipsis. Only a long result wraps below,
    // and the target keeps a minimum beside a result that fits.
    expect(rule(".chat-call")).toMatch(/flex-wrap: wrap/);
    expect(rule(".chat-call-target")).toMatch(/flex: 1 1 0(px|%)?;/);
    expect(rule(".chat-call-target")).toMatch(/min-width: var\(--layout-call-target-min\)/);
    expect(rule(".chat-call-target")).toMatch(/text-overflow: ellipsis/);
    expect(rule(".chat-call-target")).toMatch(/white-space: nowrap/);
    expect(rule(".chat-call-result")).toMatch(/max-width: 100%/);
    // A table in a report scrolls inside its own box.
    expect(rule(".markdown table")).toMatch(/display: block/);
    expect(rule(".markdown table")).toMatch(/max-width: 100%/);
    expect(rule(".markdown table")).toMatch(/overflow-x: auto/);
    expect(rule(".markdown th, .markdown td")).toMatch(/border: 1px solid var\(--border\)/);
    expect(rule(".markdown th, .markdown td")).toMatch(/padding: var\(--/);
    // The result shrinks and wraps (not flex: none), and every box in a call row can shrink below its content.
    expect(rule(".chat-call-result")).not.toMatch(/flex: none/);
    expect(rule(".chat-call-result")).toMatch(/min-width: 0/);
    expect(rule(".chat-call")).toMatch(/min-width: 0/);
    expect(rule(".chat-calls")).toMatch(/min-width: 0/);
    expect(rule(".chat-section")).toMatch(/min-width: 0/);
    // Boxes of output and code scroll or wrap inside themselves.
    expect(rule(".chat-call-output")).toMatch(/overflow: auto/);
    expect(rule(".markdown pre")).toMatch(/overflow-x: auto/);
    // The feed never scrolls on x.
    expect(rule(".chat-feed")).toMatch(/overflow-x: hidden/);
  });
});

describe("long messages fold", () => {
  const heights = (tall: string) =>
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.textContent?.includes(tall) ? 900 : 20;
    });
  afterEach(() => vi.restoreAllMocks());

  it("folds_a_tall_message_behind_read_more_and_folds_it_back", async () => {
    heights("TALL");
    start();
    page([reply("TALL report"), reply("short one")]);
    const more = screen.getByRole("button", { name: "Read more" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    expect(screen.getAllByRole("button", { name: /Read more|Show less/ })).toHaveLength(1);
    expect(screen.getByText("TALL report").closest("[data-folded]")).toHaveAttribute("data-folded", "");
    await userEvent.click(more);
    const less = screen.getByRole("button", { name: "Show less" });
    expect(less).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("TALL report").closest("[data-folded]")).toBeNull();
    await userEvent.click(less);
    expect(screen.getByRole("button", { name: "Read more" })).toBeInTheDocument();
  });

  it("a_long_system_prompt_folds_too", async () => {
    heights("TALL");
    start();
    page([prompt("TALL PROMPT.md", "system")]);
    expect(screen.getByText("TALL PROMPT.md")).toHaveClass("chat-line");
    expect(screen.getByText("TALL PROMPT.md").closest("[data-folded]")).toHaveAttribute("data-folded", "");
    await userEvent.click(screen.getByRole("button", { name: "Read more" }));
    expect(screen.getByText("TALL PROMPT.md").closest("[data-folded]")).toBeNull();
  });

  const limit = (text: string) => (screen.getByText(text).closest(".chat-fold-text") as HTMLElement).style.getPropertyValue("--fold-limit");

  it("a_system_prompt_folds_at_eight_of_its_own_lines", () => {
    heights("TALL");
    // Its own line is 17.25px (11.5px at 1.5), not the body text's 19.5px: eight of them are 138px, not 156px.
    vi.spyOn(window, "getComputedStyle").mockReturnValue({ lineHeight: "17.25px" } as CSSStyleDeclaration);
    start();
    page([prompt("TALL PROMPT.md", "system")]);
    expect(limit("TALL PROMPT.md")).toBe("138px");
    const css = readFileSync("src/chat/chat.css", "utf8");
    expect(css).toMatch(/\.chat-line\[data-from="system"\] \{[^}]*line-height: var\(--layout-chat-line\);/);
    expect(css).toMatch(/\.chat-fold-text\[data-folded\] \{[^}]*max-height: var\(--fold-limit\)/);
  });

  it("the_cut_snaps_to_the_last_whole_line_when_paragraph_margins_push_lines_down", () => {
    heights("TALL");
    vi.spyOn(window, "getComputedStyle").mockReturnValue({ lineHeight: "19.5px" } as CSSStyleDeclaration);
    // Two paragraphs: four lines, a 6px margin, then more. Line bottoms (box top at 0); the eighth ends at 162 > 156.
    const bottoms = [19.5, 39, 58.5, 78, 103.5, 123, 142.5, 162, 181.5];
    const rects = bottoms.map((bottom) => ({ top: bottom - 19.5, bottom }) as DOMRect);
    const proto = Range.prototype as unknown as { getClientRects?: () => DOMRect[] };
    const had = proto.getClientRects;
    proto.getClientRects = () => rects;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 181.5 } as DOMRect);
    try {
      start();
      page([reply("TALL one\n\nTALL two")]);
      expect(limit("TALL one")).toBe("142.5px");
    } finally {
      proto.getClientRects = had;
    }
  });

  it("a_message_is_measured_again_when_the_panel_is_resized", async () => {
    let height = 20;
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(() => height);
    const observers: { cb: ResizeObserverCallback; el: Element }[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(private cb: ResizeObserverCallback) {}
        observe(el: Element) {
          observers.push({ cb: this.cb, el });
        }
        unobserve() {}
        disconnect() {}
      },
    );
    start();
    page([reply("wraps when narrow")]);
    expect(screen.queryByRole("button", { name: "Read more" })).toBeNull();
    // The panel narrows: the text wraps onto more lines than fit.
    height = 900;
    const text = screen.getByText("wraps when narrow").closest(".chat-fold-text")!;
    const o = observers.find((x) => x.el === text)!;
    expect(o, "the fold text is observed").toBeTruthy();
    act(() => o.cb([{ contentRect: { width: 200, height: 900 } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(await screen.findByRole("button", { name: "Read more" })).toBeInTheDocument();
    // …and widens again: it fits, the button goes.
    height = 20;
    act(() => o.cb([{ contentRect: { width: 600, height: 20 } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(screen.queryByRole("button", { name: "Read more" })).toBeNull();
    vi.unstubAllGlobals();
  });

  it("a_short_message_has_no_control", () => {
    heights("TALL");
    start();
    page([reply("short one"), prompt("also short")]);
    expect(screen.queryByRole("button", { name: /Read more|Show less/ })).toBeNull();
  });
});

