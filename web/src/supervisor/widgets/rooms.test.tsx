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

const room = (id: string, title: string, over: Partial<Room> = {}): Room => ({ id, kind: "operation", title, conversation: "s1", persona: "me", unread: 0, ...over });
let next = 1;
const post = (over: Partial<Post>): Post => ({ id: next++, room: "r1", sender: "daemon", sender_name: "Fleet Manager", kind: "message", text: "", at: 1, ...over });

const ROOM_LIST = [room("r1", "Login fix", { unread: 3 }), room("r2", "Core", { kind: "product", conversation: null, unread: 2 })];
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

describe("Rooms widget", () => {
  it("the_list_shows_rooms_with_unread_counts", async () => {
    show();
    const second = await roomButton(/Core/);
    expect(second).toHaveTextContent("Product");
    expect(within(second).getByLabelText("2 unread")).toHaveTextContent("02");
    // The first room opens, and once read it has no count.
    const first = await roomButton(/Login fix/);
    expect(first).toHaveTextContent("Operation");
    await waitFor(() => expect(within(first).queryByLabelText(/unread/)).toBeNull());
    expect(ROOMS).toMatchObject({ id: "rooms", source: "rooms", minW: 4, minH: 3 });
  });

  it("clicking_a_room_opens_its_group_chat", async () => {
    const client = show();
    const first = await roomButton(/Login fix/);
    await waitFor(() => expect(first).toHaveAttribute("aria-current", "true"));
    expect(await within(await log()).findByText("on it")).toBeInTheDocument();
    await userEvent.click(await roomButton(/Core/));
    expect(client.roomPosts).toHaveBeenCalledWith("r2");
    expect(await roomButton(/Core/)).toHaveAttribute("aria-current", "true");
    expect(first).not.toHaveAttribute("aria-current");
    await waitFor(() => expect(within(screen.getByRole("log", { name: "Room posts" })).queryByText("on it")).toBeNull());
    // A new post in the open room arrives on the stream and is merged by id, once.
    const arrived = post({ room: "r2", sender: { agent: "t1" }, sender_name: "Fix login", text: "core is green" });
    const stream = FakeEventSource.instances.filter((i) => i.url.endsWith("/api/events") && !i.closed);
    act(() => stream.forEach((s) => s.emit("event", { seq: 9, kind: { type: "room_post", post: arrived } }, "9")));
    act(() => stream.forEach((s) => s.emit("event", { seq: 10, kind: { type: "room_post", post: arrived } }, "10")));
    expect(within(await log()).getAllByText("core is green")).toHaveLength(1);
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

  it("the_owner_posts_with_to_and_kind", async () => {
    const client = show();
    await within(await log()).findByText("on it");
    await userEvent.click(screen.getByRole("button", { name: "Recipient" }));
    // The room's agents only: never the owner, never the daemon, and All first.
    expect(within(menu()).getAllByRole("button").map((b) => b.textContent)).toEqual(["All", "Fix login", "supervisor"]);
    await userEvent.click(within(menu()).getByRole("button", { name: "Fix login" }));
    await userEvent.click(screen.getByRole("button", { name: "Post kind" }));
    expect(within(menu()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Message", "Announcement"]);
    await userEvent.click(within(menu()).getByRole("button", { name: "Announcement" }));
    await userEvent.type(screen.getByPlaceholderText("Post…"), "freeze main");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(client.postToRoom).toHaveBeenCalledWith("r1", { kind: "announcement", to: { agent: "t1" }, text: "freeze main" });
    expect(await within(await log()).findByText("freeze main")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Post…")).toHaveValue("");
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
