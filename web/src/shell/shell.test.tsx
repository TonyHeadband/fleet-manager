import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FakeEventSource, agent, factory, fakeClient, persona, product, task } from "../test/fixtures";
import { Footer } from "./Footer";
import { Messages, type Notice } from "./messages";

const msg = (notice: Notice, text: string, time = 0, agentId?: string) => ({ notice, text, time, agentId });

describe("Footer", () => {
  it("the_footer_shows_one_line_and_its_history", async () => {
    const m = new Messages(false);
    m.push(msg("error", "first line\nsecond line", 3600 + 5 * 60), 0);
    m.push(msg("info", "saved", 7200), 0);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} />);
    const line = screen.getByText("first line second line");
    expect(line).toHaveAttribute("title", "first line\nsecond line");
    expect(screen.getByText("1/2")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    const rows = screen.getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual(["02:00saved", "01:05first line second line"]);
    expect(screen.getByRole("button", { name: "Open log →" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Open log →" })).toHaveAttribute("title", "not on the web yet");
  });

  it("the_footer_counter_shows_the_message_on_screen", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("error", "E2"), 0);
    m.tick(8.1);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} />);
    expect(screen.getByText("E2")).toBeInTheDocument();
    expect(screen.getByText("2/2")).toBeInTheDocument();
  });

  it("the_footer_line_is_clickable_only_when_a_click_does_something", async () => {
    const m = new Messages(false);
    m.push(msg("info", "saved"), 0);
    const onClick = vi.fn();
    const { rerender } = render(<Footer messages={m} onClick={onClick} onSelect={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "saved" }));
    expect(onClick).toHaveBeenCalled();
    m.tick(4.1);
    rerender(<Footer messages={m} onClick={onClick} onSelect={() => {}} />);
    expect(screen.queryByRole("button", { name: "saved" })).not.toBeInTheDocument();
    expect(screen.getByText("saved")).toBeInTheDocument();
  });

  it("a_message_about_an_agent_selects_its_card", async () => {
    const m = new Messages(false);
    m.push(msg("error", "a1 crashed", 0, "a1"), 0);
    const onSelect = vi.fn();
    render(<Footer messages={m} onClick={() => {}} onSelect={onSelect} />);
    await userEvent.click(screen.getByRole("button", { name: "a1 crashed" }));
    expect(onSelect).toHaveBeenCalledWith("a1");
  });
});

const ready = { personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }), products: vi.fn().mockResolvedValue({ products: [product("p1", "Pie"), product("p2", "Tart")] }) };

function start(over: Parameters<typeof fakeClient>[0] = {}) {
  render(<App client={fakeClient({ ...ready, ...over })} eventSource={factory} nowSeconds={() => 1300} />);
}

function snapshot(...agents: ReturnType<typeof agent>[]) {
  act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
}

describe("Shell", () => {
  beforeEach(() => {
    FakeEventSource.reset();
    localStorage.clear();
    window.location.hash = "";
  });
  afterEach(() => vi.useRealTimers());

  it("the_drawer_holds_lites_contents_in_order", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    await within(drawer).findByRole("combobox", { name: "Active persona" });
    const order = [
      within(drawer).getByText("Fleet Manager"),
      within(drawer).getByRole("combobox", { name: "Active persona" }),
      within(drawer).getByRole("combobox", { name: "Active product" }),
      within(drawer).getByRole("button", { name: "New product" }),
      within(drawer).getByRole("button", { name: "New task" }),
      ...["Kanban", "Tasks", "Releases", "Management", "Settings"].map((v) => within(drawer).getByRole("link", { name: v })),
      within(drawer).getByText("reconnecting…"),
    ];
    for (let i = 1; i < order.length; i++) {
      expect(order[i - 1]!.compareDocumentPosition(order[i]!) & Node.DOCUMENT_POSITION_FOLLOWING, `${i}`).toBeTruthy();
    }
    expect(within(drawer).getByRole("button", { name: "New product" })).toHaveAttribute("title", "not on the web yet");
  });

  it("views_not_on_the_web_are_greyed_with_a_reason", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    expect(within(drawer).getByRole("link", { name: "Kanban" })).toHaveAttribute("aria-current", "page");
    for (const v of ["Tasks", "Releases", "Management", "Settings"]) {
      const link = within(drawer).getByRole("link", { name: v });
      expect(link).toHaveAttribute("aria-disabled", "true");
      expect(link).toHaveAttribute("title", "not on the web yet");
    }
  });

  it("the_hash_picks_the_view_and_forms_stays_reachable", async () => {
    window.location.hash = "#tasks";
    start();
    expect(await within(screen.getByRole("banner")).findByText("Kanban")).toBeInTheDocument();
    window.location.hash = "#forms";
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(screen.queryByRole("region", { name: "Tasks strip" })).not.toBeInTheDocument();
    expect(within(screen.getByRole("navigation", { name: "Drawer" })).queryByRole("link", { name: "Forms" })).not.toBeInTheDocument();
  });

  it("the_active_product_scopes_the_board_and_is_remembered", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job", product: "p1" }), agent({ id: "a2", goal: "tart job", product: "p2" }));
    const picker = await screen.findByRole("combobox", { name: "Active product" });
    await userEvent.selectOptions(picker, "Tart");
    expect(screen.getByText("tart job")).toBeInTheDocument();
    expect(screen.queryByText("pie job")).not.toBeInTheDocument();
    expect(localStorage.getItem("fleet.product")).toBe("p2");
  });

  it("a_saved_product_that_is_gone_shows_the_whole_board", async () => {
    localStorage.setItem("fleet.product", "gone");
    start();
    snapshot(agent({ id: "a1", goal: "pie job", product: "p1" }), agent({ id: "a2", goal: "tart job", product: "p2" }));
    await screen.findByRole("combobox", { name: "Active product" });
    expect(screen.getByText("pie job")).toBeInTheDocument();
    expect(screen.getByText("tart job")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New task" })).toHaveAttribute("title", "Pick a product");
  });

  it("the_strip_counts_tasks_as_lite_does", async () => {
    const tasks = vi.fn().mockResolvedValue({
      tasks: [task("r1", "running", null, 1_000_000_000), task("s1", "stopped", "stopped", 2_000_000_000), task("c1", "stopped", "crashed", 0)],
      orphans: [{ id: "o", kind: "task_folder", path: "/p/lost", worktrees: [], bytes: 500_000_000 }],
      clones_bytes: null,
    });
    start({ tasks });
    const strip = await screen.findByRole("region", { name: "Tasks strip" });
    for (const text of ["Tasks:", "1 running", "2 stopped", "1 crashed", "1 orphan", "· 3.5 GB"]) {
      expect(await within(strip).findByText(text)).toBeInTheDocument();
    }
  });

  it("the_drawer_footer_summarises_the_agents", async () => {
    start();
    snapshot(agent({ id: "a1", status: "needs_you" }), agent({ id: "a2" }), agent({ id: "a3" }));
    expect(within(screen.getByRole("navigation", { name: "Drawer" })).getByText("3 agents · 1 needs you")).toBeInTheDocument();
  });
});
