import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FakeEventSource, agent, factory, fakeClient, persona, product, task } from "../test/fixtures";
import { DetailPanel } from "./DetailPanel";
import { Footer } from "./Footer";
import { usePanel } from "./panel";
import { Messages, type Notice } from "./messages";

const msg = (notice: Notice, text: string, time = 0, agentId?: string) => ({ notice, text, time, agentId });

describe("Footer", () => {
  it("the_footer_shows_one_line_and_its_history", async () => {
    const m = new Messages(false);
    m.push(msg("error", "first line\nsecond line", 3600 + 5 * 60), 0);
    m.push(msg("info", "saved", 7200), 0);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    const line = screen.getByText("first line second line");
    expect(line).toHaveAttribute("title", "first line\nsecond line");
    expect(screen.getByText("1/2")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    const rows = screen.getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual(["02:00saved", "01:05first line second line"]);
    expect(screen.getByRole("button", { name: "Open log →" })).toBeEnabled();
  });

  it("the_history_closes_on_a_press_outside_or_escape", async () => {
    const m = new Messages(false);
    m.push(msg("info", "saved", 7200), 0);
    render(
      <>
        <p>elsewhere</p>
        <Footer messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />
      </>,
    );
    const history = screen.getByRole("button", { name: "History" });
    await userEvent.click(history);
    await userEvent.click(within(screen.getByRole("list")).getByText("saved"));
    expect(screen.getByRole("list"), "a press inside keeps it").toBeInTheDocument();
    await userEvent.click(screen.getByText("elsewhere"));
    expect(screen.queryByRole("list")).toBeNull();
    await userEvent.click(history);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("list")).toBeNull();
    expect(history).toHaveFocus();
  });

  it("the_footer_counter_shows_the_message_on_screen", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("error", "E2"), 0);
    m.tick(8.1);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(screen.getByText("E2")).toBeInTheDocument();
    expect(screen.getByText("2/2")).toBeInTheDocument();
  });

  it("the_footer_line_is_clickable_only_when_a_click_does_something", async () => {
    const m = new Messages(false);
    m.push(msg("info", "saved"), 0);
    const onClick = vi.fn();
    const { rerender } = render(<Footer messages={m} onClick={onClick} onSelect={() => {}} onOpenLog={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "saved" }));
    expect(onClick).toHaveBeenCalled();
    m.tick(4.1);
    rerender(<Footer messages={m} onClick={onClick} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(screen.queryByRole("button", { name: "saved" })).not.toBeInTheDocument();
    expect(screen.getByText("saved")).toBeInTheDocument();
  });

  it("a_message_about_an_agent_selects_its_card", async () => {
    const m = new Messages(false);
    m.push(msg("error", "a1 crashed", 0, "a1"), 0);
    const onSelect = vi.fn();
    render(<Footer messages={m} onClick={() => {}} onSelect={onSelect} onOpenLog={() => {}} />);
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

  it("the_watermark_spins", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    expect(drawer.querySelector(".watermark .radar-ring[data-spin]")).not.toBeNull();
  });

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
    // egui's New product is always there to press.
    expect(within(drawer).getByRole("button", { name: "New product" })).toBeEnabled();
  });

  it("the_first_ready_persona_becomes_the_active_one", async () => {
    start({ personas: vi.fn().mockResolvedValue({ personas: [persona("draft", false), persona("me"), persona("bot")] }) });
    const picker = await screen.findByRole("combobox", { name: "Active persona" });
    expect(picker).toHaveValue("me");
    // Saved by an effect after the render that shows it.
    await waitFor(() => expect(localStorage.getItem("fleet.persona")).toBe("me"));
  });

  it("a_saved_persona_that_is_not_ready_stays_active", async () => {
    localStorage.setItem("fleet.persona", "draft");
    start({ personas: vi.fn().mockResolvedValue({ personas: [persona("draft", false), persona("me")] }) });
    const picker = await screen.findByRole("combobox", { name: "Active persona" });
    await waitFor(() => expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toContain("me"));
    // As Lite: the picker offers ready personas only, so it asks for one, but the choice is not taken away.
    expect(picker).toHaveValue("");
    expect(localStorage.getItem("fleet.persona")).toBe("draft");
  });

  it("no_ready_persona_clears_nothing", async () => {
    localStorage.setItem("fleet.persona", "draft");
    const personas = vi.fn().mockResolvedValue({ personas: [persona("draft", false)] });
    start({ personas });
    await waitFor(() => expect(personas).toHaveBeenCalled());
    await act(async () => {});
    expect(localStorage.getItem("fleet.persona")).toBe("draft");
  });

  it("a_saved_active_persona_no_longer_listed_is_replaced", async () => {
    localStorage.setItem("fleet.persona", "gone");
    start();
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Active persona" })).toHaveValue("me"));
    await waitFor(() => expect(localStorage.getItem("fleet.persona")).toBe("me"));
  });

  it("every_view_is_on_the_web", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    expect(within(drawer).getByRole("link", { name: "Kanban" })).toHaveAttribute("aria-current", "page");
    for (const link of within(drawer).getAllByRole("link")) expect(link).not.toHaveAttribute("aria-disabled");
  });

  it("the_hash_picks_the_view_and_forms_stays_reachable", async () => {
    window.location.hash = "#nowhere";
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

/** The panel as App holds it: its width and fullscreen choice live in `usePanel`, the board's width comes from outside. */
function Panel({ boardWidth }: { boardWidth: number }) {
  const panel = usePanel();
  return (
    <DetailPanel agent={agent()} panel={panel} boardWidth={boardWidth} daemonHost="127.0.0.1" onTerminal={null} onClose={() => {}}>
      <p>pane</p>
    </DetailPanel>
  );
}

const aside = () => screen.getByRole("complementary", { name: "Detail" });

describe("DetailPanel", () => {
  it("the_header_holds_lites_buttons_in_order", () => {
    render(<Panel boardWidth={1200} />);
    const top = aside().querySelector<HTMLElement>(".detail-top")!;
    expect(within(top).getByText("core / fleet/fix-the-tests")).toHaveClass("detail-path");
    const buttons = within(top).getAllByRole("button").map((b) => b.getAttribute("aria-label"));
    expect(buttons).toEqual(["Open in terminal", "Fullscreen", "Close"]);
    expect(within(top).getByRole("button", { name: "Open in terminal" })).toBeDisabled();
    expect(within(top).getByRole("button", { name: "Open in terminal" })).toHaveAttribute("title", "the agent is stopped");
  });

  it("open_in_terminal_runs_the_attach_command", async () => {
    const onTerminal = vi.fn();
    function Running() {
      const panel = usePanel();
      return (
        <DetailPanel agent={agent({ attach: ["fleet-attach", "a1"] })} panel={panel} boardWidth={1200} daemonHost="127.0.0.1" onTerminal={onTerminal} onClose={() => {}}>
          <p>pane</p>
        </DetailPanel>
      );
    }
    render(<Running />);
    const open = within(aside()).getByRole("button", { name: "Open in terminal" });
    expect(open).toBeEnabled();
    await userEvent.click(open);
    expect(onTerminal).toHaveBeenCalled();
  });

  it("opens_at_lites_default_width", () => {
    render(<Panel boardWidth={1200} />);
    expect(aside()).toHaveStyle({ width: "420px" });
  });

  it("the_panel_drags_and_stops_at_its_minimum", () => {
    render(<Panel boardWidth={1200} />);
    const handle = screen.getByRole("separator", { name: "Resize the detail panel" });
    fireEvent.pointerDown(handle, { clientX: 780, pointerId: 1, buttons: 1 });
    fireEvent.pointerMove(handle, { clientX: 700, pointerId: 1, buttons: 1 });
    expect(aside()).toHaveStyle({ width: "500px" });
    fireEvent.pointerMove(handle, { clientX: 1190, pointerId: 1, buttons: 1 });
    fireEvent.pointerUp(handle, { clientX: 1190, pointerId: 1 });
    expect(aside()).toHaveStyle({ width: "360px" });
  });

  it("escape_leaves_a_chosen_fullscreen", async () => {
    render(<Panel boardWidth={1200} />);
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    expect(aside()).toHaveAttribute("data-layout", "full");
    await userEvent.keyboard("{Escape}");
    expect(aside()).toHaveAttribute("data-layout", "side");
  });

  it("a_narrow_window_forces_fullscreen_and_returns_to_the_chosen_width", async () => {
    const { rerender } = render(<Panel boardWidth={1000} />);
    expect(aside()).toHaveAttribute("data-layout", "side");
    rerender(<Panel boardWidth={500} />);
    expect(aside()).toHaveAttribute("data-layout", "full");
    const back = screen.getByRole("button", { name: "Back to the board" });
    expect(back).toBeDisabled();
    expect(back).toHaveAttribute("title", "the window is too narrow for the board");
    await userEvent.keyboard("{Escape}");
    expect(aside()).toHaveAttribute("data-layout", "full");
    rerender(<Panel boardWidth={1000} />);
    expect(aside()).toHaveAttribute("data-layout", "side");
    expect(aside()).toHaveStyle({ width: "420px" });
  });

  it("escape_while_typing_keeps_fullscreen", async () => {
    function Typing() {
      const panel = usePanel();
      return (
        <DetailPanel agent={agent()} panel={panel} boardWidth={1200} daemonHost="127.0.0.1" onTerminal={null} onClose={() => {}}>
          <input aria-label="Reply" />
        </DetailPanel>
      );
    }
    render(<Typing />);
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    await userEvent.click(screen.getByRole("textbox", { name: "Reply" }));
    await userEvent.keyboard("{Escape}");
    expect(aside()).toHaveAttribute("data-layout", "full");
  });

  it("a_lost_drag_does_not_resize_on_hover", () => {
    render(<Panel boardWidth={1200} />);
    const handle = screen.getByRole("separator", { name: "Resize the detail panel" });
    fireEvent.pointerDown(handle, { clientX: 780, pointerId: 1, buttons: 1 });
    fireEvent.pointerCancel(handle, { pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 700, pointerId: 1, buttons: 0 });
    expect(aside()).toHaveStyle({ width: "420px" });
    fireEvent.pointerDown(handle, { clientX: 780, pointerId: 1, buttons: 1 });
    fireEvent.pointerMove(handle, { clientX: 700, pointerId: 1, buttons: 0 });
    expect(aside()).toHaveStyle({ width: "420px" });
  });
});
