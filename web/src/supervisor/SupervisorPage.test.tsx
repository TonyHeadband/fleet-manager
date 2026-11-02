import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import type { Conversation } from "../api/generated/Conversation";
import { FakeEventSource, agent, factory, fakeClient, persona, pickInDrawer } from "../test/fixtures";
import { SupervisorPage } from "./SupervisorPage";

beforeEach(() => {
  FakeEventSource.reset();
  window.location.hash = "#supervisor";
});

const NOW = Math.floor(Date.now() / 1000);
const conv = (id: string, title: string | null, over: Partial<Conversation> = {}): Conversation => ({
  id,
  persona: "me",
  title,
  created_at: NOW,
  last_used_at: NOW,
  live: true,
  tasks: [],
  ...over,
});

/** Latest first, as the daemon lists them. */
const LIST = [conv("c2", "Chat panes"), conv("c1", "Brainstorm")];

function clientWith(lists: Record<string, Conversation[]> = { me: LIST }) {
  return fakeClient({
    conversations: vi.fn(async (p: string) => lists[p] ?? []),
    newConversation: vi.fn(async (p: string, text: string) => conv("c9", text, { persona: p })),
    sendToConversation: vi.fn().mockResolvedValue(undefined),
    chatHistory: vi.fn().mockResolvedValue({ items: [], earlier: 0 }),
    answerAsk: vi.fn().mockResolvedValue(undefined),
  });
}

const chatOf = (id: string) => FakeEventSource.instances.filter((i) => i.url.includes(`/api/agents/${id}/chat`) && !i.closed);
const page = (client = clientWith(), persona = "me") => (
  <SupervisorPage baseUrl="" client={client} persona={persona} personas={[]} agents={[]} tasks={[]} loaded factory={factory} />
);
const picker = () => screen.getByRole("button", { name: "Conversation" });

describe("Supervisor page", () => {
  it("the_drawer_lists_supervisor_first_and_routes_to_it", async () => {
    window.location.hash = "#kanban";
    render(<App client={fakeClient({ personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }) })} eventSource={factory} nowSeconds={() => NOW} />);
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    const links = within(drawer).getAllByRole("link");
    expect(links[0]).toHaveTextContent("Supervisor");
    await userEvent.click(links[0]!);
    expect(window.location.hash).toBe("#supervisor");
    expect(links[0]).toHaveAttribute("aria-current", "page");
  });

  it("opens_the_latest_conversation_of_the_persona", async () => {
    render(page());
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    expect(picker()).toHaveTextContent("Chat panes");
    expect(chatOf("c1")).toHaveLength(0);
  });

  it("names_the_owner_and_the_supervisor_in_the_chat", async () => {
    render(<SupervisorPage baseUrl="" client={clientWith()} persona="me" personas={[{ ...persona("me"), name: "Ada" }]} agents={[]} tasks={[]} loaded factory={factory} />);
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    const items = [
      { seq: 0, at: NOW, body: { type: "prompt", text: "hi", from: "owner" } },
      { seq: 1, at: NOW, body: { type: "reply", text: "hello" } },
    ];
    act(() => chatOf("c2")[0]!.emit("page", { items, earlier: 0 }));
    const names = [...document.querySelectorAll(".chat-speaker-name")].map((n) => n.textContent);
    expect(names).toEqual(["Ada", "Supervisor"]);
  });

  it("shows_the_empty_copy_for_a_persona_with_no_conversation", async () => {
    render(page(clientWith({ me: [] })));
    expect(await screen.findByText("Start a conversation with your supervisor.")).toBeInTheDocument();
    expect(FakeEventSource.instances).toHaveLength(0);
  });

  it("picking_a_conversation_switches_the_chat_and_the_url", async () => {
    render(page());
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    await userEvent.click(picker());
    await userEvent.click(within(document.querySelector<HTMLElement>(".menu")!).getByRole("button", { name: /Brainstorm/ }));
    expect(window.location.hash).toBe("#supervisor?c=c1");
    await waitFor(() => expect(chatOf("c1")).toHaveLength(1));
    expect(chatOf("c2")).toHaveLength(0);
    expect(picker()).toHaveTextContent("Brainstorm");
  });

  it("the_address_names_the_conversation", async () => {
    window.location.hash = "#supervisor?c=c1";
    render(page());
    await waitFor(() => expect(chatOf("c1")).toHaveLength(1));
    expect(chatOf("c2")).toHaveLength(0);
  });

  it("new_then_send_creates_a_conversation", async () => {
    const client = clientWith();
    render(page(client));
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    await userEvent.click(screen.getByRole("button", { name: "+ New" }));
    expect(await screen.findByText("Start a conversation with your supervisor.")).toBeInTheDocument();
    expect(chatOf("c2")).toHaveLength(0);
    expect(window.location.hash).toBe("#supervisor");
    await userEvent.type(screen.getByRole("textbox"), "hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(client.newConversation).toHaveBeenCalledWith("me", "hello");
    expect(client.sendToConversation).not.toHaveBeenCalled();
    await waitFor(() => expect(window.location.hash).toBe("#supervisor?c=c9"));
    await waitFor(() => expect(chatOf("c9")).toHaveLength(1));
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  it("send_in_an_existing_conversation_calls_sendToConversation", async () => {
    const client = clientWith();
    render(page(client));
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    await userEvent.type(screen.getByRole("textbox"), "status?");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(client.sendToConversation).toHaveBeenCalledWith("c2", "status?");
    expect(client.newConversation).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveValue(""));
  });

  it("a_failed_send_keeps_the_draft", async () => {
    const client = clientWith();
    client.sendToConversation = vi.fn().mockRejectedValue(new Error("daemon down"));
    render(page(client));
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    await userEvent.type(screen.getByRole("textbox"), "status?");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByText("daemon down")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveValue("status?");
  });

  it("switching_persona_switches_supervisor", async () => {
    const client = clientWith({ me: LIST, you: [conv("y1", "Yours", { persona: "you" })] });
    const { rerender } = render(page(client));
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    rerender(page(client, "you"));
    await waitFor(() => expect(chatOf("y1")).toHaveLength(1));
    expect(chatOf("c2")).toHaveLength(0);
    expect(client.conversations).toHaveBeenCalledWith("you");
    expect(picker()).toHaveTextContent("Yours");
  });

  it("switching_to_a_persona_with_none_shows_the_empty_page", async () => {
    window.location.hash = "#supervisor?c=c1";
    const client = clientWith({ me: LIST });
    const { rerender } = render(page(client));
    await waitFor(() => expect(chatOf("c1")).toHaveLength(1));
    rerender(page(client, "you"));
    expect(await screen.findByText("Start a conversation with your supervisor.")).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe("#supervisor"));
  });

  it("the_grid_section_holds_the_agents_storage_and_rooms_widgets", async () => {
    const { container } = render(page());
    await waitFor(() => expect(chatOf("c2")).toHaveLength(1));
    const grid = container.querySelector(".supervisor-grid");
    expect(grid).toHaveClass("outlined");
    expect([...grid!.querySelectorAll(".widget")].map((w) => w.getAttribute("data-widget"))).toEqual(["agents_by_status", "storage", "rooms"]);
    // The chat stays on the left and the grid fills the space beside it, never covering it.
    expect(container.querySelector(".supervisor-chat")!.contains(grid)).toBe(false);
    expect([...container.querySelector(".supervisor")!.children].map((c) => c.className)).toEqual(["supervisor-chat outlined", "supervisor-grid outlined"]);
  });

  it("each_widget_keeps_the_size_the_grid_gives_it", () => {
    const css = readFileSync("src/supervisor/supervisor.css", "utf8").replace(/\s+/g, " ");
    // #491: two rows share the grid's height, the graphs on top and Rooms across the bottom; a cell never grows.
    expect(css).not.toMatch(/\.supervisor \{[^}]*flex-direction: column;/);
    expect(css).toMatch(/\.supervisor-grid \{[^}]*flex: 1;[^}]*display: grid; grid-template-columns: repeat\(2, minmax\(0, 1fr\)\); grid-template-rows: repeat\(2, minmax\(var\(--layout-panel-min\), 1fr\)\);/);
    expect(css).toMatch(/\.supervisor-cell \{[^}]*min-height: 0;[^}]*overflow: hidden;/);
    expect(css).toMatch(/\.supervisor-cell\[data-cell="rooms"\] \{[^}]*grid-column: 1 \/ -1;/);
    const rooms = readFileSync("src/supervisor/widgets/rooms.css", "utf8").replace(/\s+/g, " ");
    expect(rooms).not.toMatch(/\.rooms \.list-detail \{[^}]*height: auto/);
    expect(rooms).toMatch(/\.rooms \.list-detail \{[^}]*flex: 1;[^}]*min-height: 0;/);
  });

  it("supervisor_agents_do_not_appear_on_the_kanban", async () => {
    window.location.hash = "#kanban";
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => NOW} />);
    await screen.findByRole("region", { name: "Working" });
    act(() =>
      FakeEventSource.for("/api/events").emit(
        "snapshot",
        { seq: 1, agents: [agent({ id: "s1", kind: "supervisor", goal: "supervising" }), agent({ id: "a1", goal: "fix the tests" })] },
        "1",
      ),
    );
    expect(await screen.findByText("fix the tests")).toBeInTheDocument();
    expect(screen.queryByText("supervising")).toBeNull();
  });

  it("supervisor_agents_do_not_count_in_the_footer", async () => {
    window.location.hash = "#kanban";
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => NOW} />);
    await screen.findByRole("region", { name: "Working" });
    act(() =>
      FakeEventSource.for("/api/events").emit(
        "snapshot",
        { seq: 1, agents: [agent({ id: "s1", kind: "supervisor", status: "needs_you" }), agent({ id: "a1", goal: "fix the tests" })] },
        "1",
      ),
    );
    await screen.findByText("fix the tests");
    expect(screen.getByLabelText("Status")).toHaveTextContent("Agents01");
    expect(screen.queryByText("Need you")).toBeNull();
  });

  it("supervisor_css_uses_tokens_only", () => {
    const css = readFileSync("src/supervisor/supervisor.css", "utf8");
    expect(css).toContain(".supervisor");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/gradient|border-radius/);
  });
});
