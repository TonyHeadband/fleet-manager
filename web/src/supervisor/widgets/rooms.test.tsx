import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../../api/client";
import type { Post } from "../../api/generated/Post";
import type { Room } from "../../api/generated/Room";
import { FakeEventSource, agent, factory, fakeClient } from "../../test/fixtures";
import { ROOMS } from "./Rooms";

beforeEach(() => FakeEventSource.reset());

const room = (id: string, title: string, over: Partial<Room> = {}): Room => ({ id, kind: "operation", title, conversation: "s1", persona: "me", unread: 0, members: [], ...over });
let next = 1;
const post = (over: Partial<Post>): Post => ({ id: next++, room: "r1", sender: "daemon", sender_name: "Fleet Manager", kind: "message", text: "", at: 1, ...over });

// Write docs is in the room but has not posted.
const MEMBERS = [
  { id: "t1", name: "Fix login" },
  { id: "s1", name: "supervisor" },
  { id: "t2", name: "Write docs" },
];
const ROOM_LIST = [room("r1", "Login fix", { unread: 3, members: MEMBERS }), room("r2", "Core", { kind: "product", conversation: null, unread: 2 })];
const FROM_TASK = post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "on it" });
const FROM_SUPERVISOR = post({ sender: { agent: "s1" }, sender_name: "supervisor", text: "go ahead" });
const FROM_OWNER = post({ sender: "owner", sender_name: "owner", text: "thanks" });
const FROM_DAEMON = post({ kind: "announcement", text: "PR #4 landed" });
const AGENTS = [agent({ id: "t1", kind: "orchestrator", role: "direct", persona: "me" }), agent({ id: "s1", kind: "supervisor", persona: "me" })];

function clientWith(posts: Record<string, Post[]> = { r1: [FROM_TASK, FROM_SUPERVISOR, FROM_OWNER, FROM_DAEMON], r2: [] }, over = {}) {
  return fakeClient({
    rooms: vi.fn().mockResolvedValue(ROOM_LIST),
    roomPosts: vi.fn(async (id: string) => posts[id] ?? []),
    postToRoom: vi.fn(async (id: string, body: { kind: Post["kind"]; text: string }) => post({ room: id, sender: "owner", sender_name: "owner", ...body, to: null })),
    answerRoomAsk: vi.fn().mockResolvedValue(undefined),
    ...over,
  });
}

const show = (client = clientWith()) => {
  render(ROOMS.render({ persona: "me", agents: AGENTS, tasks: [], loaded: true, baseUrl: "", client, factory }));
  return client;
};
const roomButton = (name: RegExp) => screen.findByRole("button", { name });
const log = () => screen.findByRole("log", { name: "Room posts" });
const menu = () => document.querySelector<HTMLElement>(".menu")!;
const streams = () => FakeEventSource.instances.filter((i) => i.url.endsWith("/api/events") && !i.closed);
const emit = (...posts: Post[]) => act(() => posts.forEach((p, i) => streams().forEach((s) => s.emit("event", { seq: 100 + i, kind: { type: "room_post", post: p } }, String(100 + i)))));
/** The trailing read after live posts waits this long; the tests wait past it. */
const SETTLE = 700;
const settle = () => act(() => new Promise((r) => setTimeout(r, SETTLE)));

describe("Rooms widget", () => {
  it("a_room_picked_in_the_list_is_the_one_the_address_names_so_a_later_toast_reopens_another", async () => {
    window.location.hash = "#supervisor?c=s1&room=r2";
    const before = window.history.length;
    show();
    expect(await screen.findByRole("region", { name: "Core" })).toBeInTheDocument();
    await userEvent.click(await roomButton(/Login fix/));
    expect(await screen.findByRole("region", { name: "Login fix" })).toBeInTheDocument();
    // Replaced, not pushed, and in step with the pick.
    expect(window.location.hash).toBe("#supervisor?c=s1&room=r1");
    expect(window.history.length).toBe(before);
    // A toast for Core links to the same address as before the pick: it must open Core again.
    act(() => {
      window.location.hash = "#supervisor?c=s1&room=r2";
    });
    expect(await screen.findByRole("region", { name: "Core" })).toBeInTheDocument();
    window.location.hash = "";
  });

  it("an_operation_room_opens_by_its_id", async () => {
    window.location.hash = "#supervisor?c=s1&room=r1";
    show();
    expect(await screen.findByRole("region", { name: "Login fix" })).toBeInTheDocument();
    window.location.hash = "";
  });

  it("the_room_the_address_names_opens", async () => {
    window.location.hash = "#supervisor?c=s1&room=r2";
    show();
    expect(await screen.findByRole("region", { name: "Core" })).toBeInTheDocument();
    window.location.hash = "";
  });

  it("the_list_shows_rooms_with_unread_counts", async () => {
    show();
    const second = await roomButton(/Core/);
    expect(second).toHaveTextContent("Product");
    expect(second).toHaveTextContent("02");
    expect(within(second).getByText("2 unread")).toHaveClass("visually-hidden");
    // The first room opens, and once read it has no count.
    const first = await roomButton(/Login fix/);
    expect(first).toHaveTextContent("Operation");
    await waitFor(() => expect(within(first).queryByText(/unread/)).toBeNull());
    expect(ROOMS).toMatchObject({ id: "rooms", source: "rooms", minW: 4, minH: 3 });
  });

  it("clicking_a_room_opens_its_group_chat", async () => {
    const client = show();
    const first = await roomButton(/Login fix/);
    await waitFor(() => expect(first).toHaveAttribute("aria-current", "true"));
    expect(await within(await log()).findByText("on it")).toBeInTheDocument();
    const before = streams();
    await userEvent.click(await roomButton(/Core/));
    expect(client.roomPosts).toHaveBeenCalledWith("r2");
    // The previous room's stream is closed: one stream stays open, the new room's.
    expect(before.every((s) => s.closed)).toBe(true);
    expect(streams()).toHaveLength(1);
    expect(await roomButton(/Core/)).toHaveAttribute("aria-current", "true");
    expect(first).not.toHaveAttribute("aria-current");
    await waitFor(() => expect(within(screen.getByRole("log", { name: "Room posts" })).queryByText("on it")).toBeNull());
    // A new post in the open room arrives on the stream and is merged by id, once.
    const arrived = post({ room: "r2", sender: { agent: "t1" }, sender_name: "Fix login", text: "core is green" });
    emit(arrived);
    emit(arrived);
    expect(within(await log()).getAllByText("core is green")).toHaveLength(1);
  });

  it("live_posts_in_the_open_room_are_marked_read_once_per_burst", async () => {
    const client = show();
    await within(await log()).findByText("on it");
    const reads = () => vi.mocked(client.roomPosts).mock.calls.filter(([id]) => id === "r1").length;
    expect(reads()).toBe(1);
    emit(post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "one" }));
    await waitFor(() => expect(reads()).toBe(2), { timeout: 2000 });
    emit(...["a", "b", "c"].map((text) => post({ sender: { agent: "t1" }, sender_name: "Fix login", text })));
    await settle();
    expect(reads()).toBe(3);
    // Another room's post reads nothing here.
    emit(post({ room: "r2", sender: { agent: "t1" }, sender_name: "Fix login", text: "elsewhere" }));
    await settle();
    expect(reads()).toBe(3);
  });

  it("a_read_asked_while_one_is_in_flight_runs_once_after_it", async () => {
    let release: (ps: Post[]) => void = () => {};
    let r1Calls = 0;
    const roomPosts = vi.fn((id: string) => {
      if (id !== "r1") return Promise.resolve<Post[]>([]);
      r1Calls += 1;
      if (r1Calls === 2) return new Promise<Post[]>((r) => (release = r));
      return Promise.resolve([FROM_TASK]);
    });
    show(clientWith(undefined, { roomPosts }));
    await within(await log()).findByText("on it");
    const reads = () => roomPosts.mock.calls.filter(([id]) => id === "r1").length;
    emit(post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "one" }));
    await waitFor(() => expect(reads()).toBe(2), { timeout: 2000 });
    // Two more bursts while that read is out: they ask for one read after it, not two and not none.
    emit(post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "two" }));
    await settle();
    emit(post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "three" }));
    await settle();
    expect(reads()).toBe(2);
    await act(async () => release([FROM_TASK]));
    await waitFor(() => expect(reads()).toBe(3));
    await settle();
    expect(reads()).toBe(3);
  });

  it("a_pending_read_is_flushed_when_the_room_closes", async () => {
    const client = show();
    await within(await log()).findByText("on it");
    emit(post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "last words" }));
    await userEvent.click(await roomButton(/Core/));
    expect(vi.mocked(client.roomPosts).mock.calls.map(([id]) => id)).toEqual(["r1", "r1", "r2"]);
    await settle();
    expect(client.roomPosts).toHaveBeenCalledTimes(3);
  });

  it("senders_show_name_and_agent_avatar", async () => {
    show();
    const feed = await log();
    const reply = (await within(feed).findByText("on it")).closest(".chat-reply")!;
    expect(reply.querySelector(".chat-speaker")).toHaveTextContent("Fix login");
    expect(reply.querySelector(".avatar")).toHaveAttribute("data-kind", "specialist");
    expect(reply.querySelector(".avatar")).toHaveAttribute("data-seed", "direct");
    const supervisor = within(feed).getByText("go ahead").closest(".chat-reply")!;
    expect(supervisor.querySelector(".avatar")).toHaveAttribute("data-seed", "supervisor:me");
    const owner = within(feed).getByText("thanks").closest(".chat-prompt")!;
    expect(owner).toHaveAttribute("data-from", "owner");
    expect(owner.querySelector(".avatar")).toHaveAttribute("data-kind", "persona");
    expect(owner.querySelector(".avatar")).toHaveAttribute("data-seed", "me");
    const daemon = within(feed).getByText(/PR #4 landed/);
    expect(daemon).toHaveClass("chat-line");
    expect(daemon).toHaveAttribute("data-from", "system");
    expect(daemon.querySelector(".avatar")).toBeNull();
  });

  it("a_task_notice_announcement_reads_as_a_system_line", async () => {
    show();
    const feed = await log();
    await within(feed).findByText("on it");
    emit(post({ kind: "announcement", text: "Fix login · checks passed" }));
    const line = await within(feed).findByText(/Fix login · checks passed/);
    expect(line).toHaveClass("chat-line");
    expect(line).toHaveAttribute("data-from", "system");
    expect(line.querySelector(".avatar")).toBeNull();
  });

  it("a_supervisor_is_known_by_its_kind_in_a_product_room", async () => {
    const ask = post({ room: "r2", sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: { agent: "s1" }, text: "Merge order?" });
    const said = post({ room: "r2", sender: { agent: "s1" }, sender_name: "supervisor", text: "core first" });
    const client = clientWith({ r1: [], r2: [ask, said] });
    show(client);
    await userEvent.click(await roomButton(/Core/));
    const feed = await log();
    const reply = (await within(feed).findByText("core first")).closest(".chat-reply")!;
    expect(reply.querySelector(".avatar")).toHaveAttribute("data-seed", "supervisor:me");
    // An ask to the supervisor is the owner's to answer too, in a product room as well.
    const group = within(feed).getByText("Merge order?").closest<HTMLElement>(".chat-ask")!;
    expect(within(group).getByRole("textbox", { name: "Answer" })).toBeInTheDocument();
  });

  it("the_owners_announcement_reads_as_an_announcement", async () => {
    show(clientWith({ r1: [post({ sender: "owner", sender_name: "owner", kind: "announcement", text: "freeze main" })] }));
    const line = await within(await log()).findByText(/freeze main/);
    expect(line).toHaveClass("chat-line");
    expect(line).toHaveTextContent("owner · freeze main");
    expect(line.closest(".chat-prompt")).toBeNull();
  });

  it("the_owner_posts_with_to_and_kind", async () => {
    const client = show();
    await within(await log()).findByText("on it");
    await userEvent.click(screen.getByRole("button", { name: "Recipient" }));
    // The room's members, whether or not they have posted (never the owner or the daemon), and All first.
    expect(within(menu()).getAllByRole("button").map((b) => b.textContent)).toEqual(["All", "Fix login", "supervisor", "Write docs"]);
    await userEvent.click(within(menu()).getByRole("button", { name: "Fix login" }));
    await userEvent.click(screen.getByRole("button", { name: "Post kind" }));
    expect(within(menu()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Message", "Announcement"]);
    await userEvent.click(within(menu()).getByRole("button", { name: "Announcement" }));
    await userEvent.type(screen.getByPlaceholderText("Post…"), "freeze main");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(client.postToRoom).toHaveBeenCalledWith("r1", { kind: "announcement", to: { agent: "t1" }, text: "freeze main" });
    // An announcement, the owner's too, reads as a system line.
    expect(await within(await log()).findByText("owner · freeze main")).toHaveClass("chat-line");
    expect(screen.getByPlaceholderText("Post…")).toHaveValue("");
  });

  it("an_answered_ask_leaves_the_accent_and_an_open_one_keeps_it", async () => {
    const open = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Which port?" });
    const done = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Which host?", answered_at: 1767225600 });
    show(clientWith({ r1: [open, done] }));
    const feed = await log();
    expect((await within(feed).findByText("Which port?")).closest(".chat-ask")).not.toHaveAttribute("data-answered");
    expect(within(feed).getByText("Which host?").closest(".chat-ask")).toHaveAttribute("data-answered");
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-ask\[data-answered\] \{[^}]*border-color: var\(--text\);/);
    expect(css).toMatch(/\.chat-ask\[data-answered\] \.chat-ask-caption \{[^}]*color: var\(--text\);/);
  });

  describe("long posts fold (#487 in rooms)", () => {
    const heights = () =>
      vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
        return this.textContent?.includes("TALL") ? 900 : 20;
      });

    it("a_long_announcement_and_message_fold_and_open", async () => {
      heights();
      const long = post({ kind: "announcement", text: "TALL announcement" });
      const said = post({ sender: { agent: "t1" }, sender_name: "Fix login", text: "TALL message" });
      show(clientWith({ r1: [long, said] }));
      const feed = await log();
      await within(feed).findByText(/TALL announcement/);
      const more = within(feed).getAllByRole("button", { name: "Read more" });
      expect(more).toHaveLength(2);
      await userEvent.click(more[0]!);
      expect(within(feed).getByRole("button", { name: "Show less" })).toBeInTheDocument();
      expect(within(feed).getByText(/TALL announcement/).closest("[data-folded]")).toBeNull();
      vi.restoreAllMocks();
    });

    it("a_long_answer_under_its_ask_folds_and_is_not_one_line", async () => {
      heights();
      const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Which findings?" });
      const answer = post({ sender: "owner", sender_name: "owner", kind: "answer", reply_to: ask.id, text: "TALL answer\n\n1. first finding" });
      show(clientWith({ r1: [ask, answer] }));
      const feed = await log();
      const group = (await within(feed).findByText("Which findings?")).closest<HTMLElement>(".chat-ask")!;
      // The answer is its own folded markdown block under a short "Answered" caption, not one long caption line.
      expect(within(group).getByText("Answered")).toBeInTheDocument();
      expect(within(group).getByText("first finding")).toBeInTheDocument();
      await userEvent.click(within(group).getByRole("button", { name: "Read more" }));
      expect(within(group).getByRole("button", { name: "Show less" })).toBeInTheDocument();
      vi.restoreAllMocks();
    });
  });

  it("an_owner_ask_answers_inline", async () => {
    const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Which port?" });
    const toSomeoneElse = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: { agent: "t9" }, text: "Ready?" });
    const client = show(clientWith({ r1: [ask, toSomeoneElse] }));
    const group = (await within(await log()).findByText("Which port?")).closest<HTMLElement>(".chat-ask")!;
    expect(within(group).getByText("Asks")).toBeInTheDocument();
    expect(group.querySelector(".chat-speaker")).toHaveTextContent("Fix login");
    // An ask to another member has no box here.
    expect(within(screen.getByText("Ready?").closest<HTMLElement>(".chat-ask")!).queryByRole("textbox")).toBeNull();
    await userEvent.type(within(group).getByRole("textbox", { name: "Answer" }), "8080");
    await userEvent.click(within(group).getByRole("button", { name: "Send" }));
    expect(client.answerRoomAsk).toHaveBeenCalledWith(ask.id, { text: "8080" });
    expect(await within(group).findByText("Answered · 8080")).toBeInTheDocument();
    expect(within(group).queryByRole("textbox")).toBeNull();
  });

  it("a_breakdown_ask_shows_approve_and_request_changes", async () => {
    const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: { agent: "s1" }, text: "Approve breakdown v2?", breakdown_version: 2 });
    const client = show(clientWith({ r1: [ask] }));
    const group = (await within(await log()).findByText("Approve breakdown v2?")).closest<HTMLElement>(".chat-ask")!;
    expect(within(group).queryByRole("button", { name: "Send" })).toBeNull();
    const changes = within(group).getByRole("button", { name: "Request changes" });
    expect(changes).toBeDisabled();
    await userEvent.type(within(group).getByRole("textbox", { name: "Changes" }), "split step 3");
    await userEvent.click(changes);
    expect(client.answerRoomAsk).toHaveBeenCalledWith(ask.id, { text: "split step 3" });
    expect(await within(group).findByText("Answered · split step 3")).toBeInTheDocument();
  });

  it("a_breakdown_ask_approves", async () => {
    const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Approve breakdown v1?", breakdown_version: 1 });
    const client = show(clientWith({ r1: [ask] }));
    const group = (await within(await log()).findByText("Approve breakdown v1?")).closest<HTMLElement>(".chat-ask")!;
    await userEvent.click(within(group).getByRole("button", { name: "Approve" }));
    expect(client.answerRoomAsk).toHaveBeenCalledWith(ask.id, { approve: true });
    expect(await within(group).findByText("Answered · Approved")).toBeInTheDocument();
  });

  it("a_second_answer_shows_already_answered", async () => {
    const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: "owner", text: "Which port?" });
    const answerRoomAsk = vi.fn().mockRejectedValue(new FleetError("already answered", undefined, undefined, 409));
    show(clientWith({ r1: [ask] }, { answerRoomAsk }));
    const group = (await within(await log()).findByText("Which port?")).closest<HTMLElement>(".chat-ask")!;
    await userEvent.type(within(group).getByRole("textbox", { name: "Answer" }), "8080");
    await userEvent.click(within(group).getByRole("button", { name: "Send" }));
    expect(await within(group).findByText("already answered")).toBeInTheDocument();
  });

  it("an_answer_post_shows_under_its_ask", async () => {
    const ask = post({ sender: { agent: "t1" }, sender_name: "Fix login", kind: "ask", to: { agent: "s1" }, text: "Which port?", answered_at: 2 });
    const answer = post({ sender: { agent: "s1" }, sender_name: "supervisor", kind: "answer", reply_to: ask.id, text: "9090" });
    show(clientWith({ r1: [ask, answer] }));
    const group = (await within(await log()).findByText("Which port?")).closest<HTMLElement>(".chat-ask")!;
    expect(within(group).getByText("Answered · 9090")).toBeInTheDocument();
    expect(within(group).queryByRole("textbox")).toBeNull();
    expect(screen.getAllByText(/9090/)).toHaveLength(1);
  });

  it("rooms_css_uses_tokens_only", () => {
    const css = readFileSync("src/supervisor/widgets/rooms.css", "utf8");
    expect(css).toContain(".rooms");
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/border-radius|gradient/);
    expect([...css.matchAll(/(?<![\w-])\d*\.?\d+(px|rem|em)\b/g)].map((m) => m[0]).filter((v) => v !== "1px")).toEqual([]);
    // The widget wipes in under the motion guard only.
    expect(css).toMatch(/prefers-reduced-motion: no-preference/);
    expect(css).toMatch(/:root:not\(\[data-animations="off"\]\) \.widget\[data-widget="rooms"\] \{ animation: fui-wipe /);
  });
});
