import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FakeEventSource, agent, factory, fakeClient, persona, pickInDrawer, product, task } from "../test/fixtures";
import { DetailPanel } from "./DetailPanel";
import { Footer } from "./Footer";
import { usePanel } from "./panel";
import { layout } from "../generated/layout";
import { Messages, type Notice } from "./messages";

const msg = (notice: Notice, text: string, time = 0, agentId?: string) => ({ notice, text, time, agentId });

const rail = { connected: true, agents: [] };
const keys = () => [...document.querySelectorAll(".footer .readout-key")].map((k) => k.textContent);

describe("Footer", () => {
  it("the_footer_is_a_rail", () => {
    const m = new Messages(false);
    m.push(msg("error", "first", 0), 0);
    m.push(msg("info", "second", 0), 0);
    const agents = [agent({ id: "a1", status: "needs_you" }), agent({ id: "a2", status: "needs_you" }), ...Array.from({ length: 7 }, (_, n) => agent({ id: `b${n}` }))];
    render(<Footer connected agents={agents} messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(keys()).toEqual(["Link", "Agents", "Need you", "Sel"]);
    expect(screen.getByText("09")).toHaveAttribute("aria-label", "9");
    // Across every product, unlike the header's task counts: the title says so.
    expect(screen.getByText("09").closest(".readout")).toHaveAttribute("title", "Every agent the daemon runs, in every product");
    expect(screen.getByText("live")).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Status" }).lastElementChild).toHaveTextContent("History");
  });

  it("need_you_shows_only_when_someone_does", () => {
    render(<Footer connected={false} agents={[agent({ id: "a1" })]} messages={new Messages(false)} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(keys()).toEqual(["Link", "Agents"]);
    expect(screen.getByText("reconnecting…")).toBeInTheDocument();
  });

  it("the_message_cell_keeps_its_notice_tone", async () => {
    const m = new Messages(false);
    m.push(msg("error", "broke", 0, "a1"), 0);
    const onSelect = vi.fn();
    render(<Footer {...rail} messages={m} onClick={() => {}} onSelect={onSelect} onOpenLog={() => {}} />);
    const cell = screen.getByText("broke").closest(".footer-line")!;
    expect(cell.querySelector('.dot[data-notice="error"]')).not.toBeNull();
    await userEvent.click(screen.getByText("broke"));
    expect(onSelect).toHaveBeenCalledWith("a1");
  });

  it("the_footer_shows_one_line_and_its_history", async () => {
    const m = new Messages(false);
    m.push(msg("error", "first line\nsecond line", 3600 + 5 * 60), 0);
    m.push(msg("info", "saved", 7200), 0);
    render(<Footer {...rail} messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    const line = screen.getByText("first line second line");
    expect(line).toHaveAttribute("title", "first line\nsecond line");
    expect(screen.getByText("01/02")).toHaveAttribute("aria-label", "1/2");
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
        <Footer {...rail} messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />
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
    render(<Footer {...rail} messages={m} onClick={() => {}} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(screen.getByText("E2")).toBeInTheDocument();
    expect(screen.getByText("02/02")).toBeInTheDocument();
  });

  it("the_footer_line_is_clickable_only_when_a_click_does_something", async () => {
    const m = new Messages(false);
    m.push(msg("info", "saved"), 0);
    const onClick = vi.fn();
    const { rerender } = render(<Footer {...rail} messages={m} onClick={onClick} onSelect={() => {}} onOpenLog={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "saved" }));
    expect(onClick).toHaveBeenCalled();
    m.tick(4.1);
    rerender(<Footer {...rail} messages={m} onClick={onClick} onSelect={() => {}} onOpenLog={() => {}} />);
    expect(screen.queryByRole("button", { name: "saved" })).not.toBeInTheDocument();
    expect(screen.getByText("saved")).toBeInTheDocument();
  });

  it("a_message_about_an_agent_selects_its_card", async () => {
    const m = new Messages(false);
    m.push(msg("error", "a1 crashed", 0, "a1"), 0);
    const onSelect = vi.fn();
    render(<Footer {...rail} messages={m} onClick={() => {}} onSelect={onSelect} onOpenLog={() => {}} />);
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

  it("the_watermark_spins_slowly", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    const ring = drawer.querySelector<SVGGElement>(".watermark .radar-ring[data-spin]");
    expect(ring).not.toBeNull();
    // Slower than the boot mark's 15 s: in the drawer it is background.
    expect(ring!.style.animationDuration).toBe("30000ms");
    // Its dots are drawn larger than the boot mark's, or they vanish at the drawer's scale.
    expect(ring!.querySelector("circle + circle")).toHaveAttribute("r", "0.7");
  });

  it("the_board_top_bar_is_titled_filters", async () => {
    // The drawer already names the view; the board's top bar holds its filters.
    start();
    expect(await within(screen.getByRole("banner")).findByRole("heading", { name: "Filters" })).toBeInTheDocument();
  });

  it("the_drawer_views_are_rail_labels", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    for (const link of within(drawer).getAllByRole("link")) expect(link.querySelector(".marker")).not.toBeNull();
  });

  it("the_drawer_holds_lites_contents_in_order", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    await within(drawer).findByRole("button", { name: "Active persona" });
    const order = [
      within(drawer).getByText("Fleet Manager"),
      within(drawer).getByRole("button", { name: "Active persona" }),
      within(drawer).getByRole("button", { name: "Active product" }),
      within(drawer).getByRole("button", { name: "New product" }),
      within(drawer).getByRole("button", { name: "New task" }),
      ...["Kanban", "Tasks", "Releases", "Fleet", "Settings"].map((v) => within(drawer).getByRole("link", { name: v })),
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
    const picker = await screen.findByRole("button", { name: "Active persona" });
    expect(picker).toHaveTextContent("me");
    // Saved by an effect after the render that shows it.
    await waitFor(() => expect(localStorage.getItem("fleet.persona")).toBe("me"));
  });

  it("a_saved_persona_that_is_not_ready_stays_active", async () => {
    localStorage.setItem("fleet.persona", "draft");
    start({ personas: vi.fn().mockResolvedValue({ personas: [persona("draft", false), persona("me")] }) });
    const picker = await screen.findByRole("button", { name: "Active persona" });
    // As Lite: the picker offers ready personas only, so it asks for one, but the choice is not taken away.
    expect(picker).toHaveTextContent("Pick a persona");
    await userEvent.click(picker);
    expect(within(document.querySelector<HTMLElement>(".menu")!).getAllByRole("button").map((b) => b.textContent)).toEqual(["me"]);
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
    await waitFor(() => expect(screen.getByRole("button", { name: "Active persona" })).toHaveTextContent("me"));
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
    expect(await within(screen.getByRole("banner")).findByText("Filters")).toBeInTheDocument();
    window.location.hash = "#forms";
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(screen.queryByRole("group", { name: "Tasks" })).not.toBeInTheDocument();
    expect(within(screen.getByRole("navigation", { name: "Drawer" })).queryByRole("link", { name: "Forms" })).not.toBeInTheDocument();
  });

  it("the_active_product_scopes_the_board_and_is_remembered", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job", product: "p1" }), agent({ id: "a2", goal: "tart job", product: "p2" }));
    await pickInDrawer("Active product", "Tart");
    expect(screen.getByText("tart job")).toBeInTheDocument();
    expect(screen.queryByText("pie job")).not.toBeInTheDocument();
    expect(localStorage.getItem("fleet.product")).toBe("p2");
  });

  it("a_saved_product_that_is_gone_shows_the_whole_board", async () => {
    localStorage.setItem("fleet.product", "gone");
    start();
    snapshot(agent({ id: "a1", goal: "pie job", product: "p1" }), agent({ id: "a2", goal: "tart job", product: "p2" }));
    await screen.findByRole("button", { name: "Active product" });
    expect(screen.getByText("pie job")).toBeInTheDocument();
    expect(screen.getByText("tart job")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New task" })).toHaveAttribute("title", "Pick a product");
  });

  it("the_task_counts_are_a_header_rail", async () => {
    const tasks = vi.fn().mockResolvedValue({
      tasks: [task("r1", "running", null, 1_000_000_000), task("s1", "stopped", "stopped", 2_000_000_000), task("c1", "stopped", "crashed", 0)],
      orphans: [{ id: "o", kind: "task_folder", path: "/p/lost", worktrees: [], bytes: 500_000_000 }],
      clones_bytes: null,
    });
    start({ tasks });
    const rail = await within(screen.getByRole("banner")).findByRole("group", { name: "Tasks" });
    expect(screen.queryByRole("region", { name: "Tasks strip" })).toBeNull();
    for (const text of ["03", "01", "02", "3.5 GB"]) expect(within(rail).getAllByText(text).length).toBeGreaterThan(0);
  });

  it("every_view_sits_on_the_canvas", async () => {
    start();
    // Fleet and Settings sit in the same Shell; their own tests cover what they load.
    for (const hash of ["#kanban", "#tasks", "#releases", "#forms"]) {
      window.location.hash = hash;
      act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
      expect(await screen.findByRole("banner")).toHaveClass("outlined");
      expect(screen.getByRole("main")).toHaveClass("outlined");
      expect(screen.getByRole("contentinfo")).toHaveClass("outlined");
    }
  });

  it("the_drawer_footer_shows_only_the_live_status", async () => {
    start();
    act(() => FakeEventSource.for("/api/events").emit("open", null));
    snapshot(agent({ id: "a1", status: "needs_you" }), agent({ id: "a2" }), agent({ id: "a3" }));
    // The daemon's connection, not the fleet: the agents are counted on the board.
    const footer = (await screen.findByRole("navigation", { name: "Drawer" })).querySelector(".drawer-footer");
    expect(footer).toHaveTextContent(/^live$/);
  });

  it("pressing_the_selected_card_closes_the_panel", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(await screen.findByText("pie job"));
    expect(screen.getByRole("complementary", { name: "Detail" })).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("main")).getByText("pie job"));
    // It pulls out first, then goes (push.test.tsx).
    fireEvent.animationEnd(screen.getByRole("complementary", { name: "Detail" }), { animationName: "fui-pull" });
    expect(screen.queryByRole("complementary", { name: "Detail" })).toBeNull();
  });

  it("a_click_on_empty_board_closes_the_panel", async () => {
    start();
    snapshot(agent({ id: "a1", goal: "pie job" }));
    await userEvent.click(await screen.findByText("pie job"));
    await userEvent.click(screen.getByRole("region", { name: "Review" }).querySelector(".column-body")!);
    fireEvent.animationEnd(screen.getByRole("complementary", { name: "Detail" }), { animationName: "fui-pull" });
    expect(screen.queryByRole("complementary", { name: "Detail" })).toBeNull();
  });

  it("the_theme_switch_sits_by_the_app_name", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    const theme = within(drawer).getByRole("group", { name: "Theme" });
    expect(theme.closest(".drawer-header")).toHaveTextContent("Fleet Manager");
    expect(within(screen.getByRole("banner")).queryByRole("group", { name: "Theme" })).toBeNull();
  });

  it("each_view_has_a_marker_on_the_rail", async () => {
    start();
    const drawer = await screen.findByRole("navigation", { name: "Drawer" });
    const kanban = within(drawer).getByRole("link", { name: "Kanban" });
    expect(kanban.querySelector(".marker[aria-hidden='true']")).not.toBeNull();
    expect(kanban).not.toHaveClass("button");
  });
});

/** The panel as App holds it: its fullscreen choice lives in `usePanel`, the body's width comes from outside. */
function Panel({ boardWidth, wide = false, onCover }: { boardWidth: number; wide?: boolean; onCover?: (covering: boolean) => void }) {
  const panel = usePanel();
  return (
    <DetailPanel
      agent={agent()}
      panel={{ ...panel, wide: panel.wide || wide }}
      boardWidth={boardWidth}
     
      daemonHost="127.0.0.1"
      onTerminal={null}
      onClose={() => {}}
      onCover={onCover}
    >
      <p>pane</p>
    </DetailPanel>
  );
}

const aside = () => screen.getByRole("complementary", { name: "Detail" });
const end = (name: string) => fireEvent.animationEnd(aside(), { animationName: name });

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

  it("opens_at_a_phones_width", () => {
    render(<Panel boardWidth={1222} />);
    expect(aside()).toHaveStyle({ width: `${layout.panel}px` });
    // Its width follows the board's columns: nothing to drag.
    expect(screen.queryByRole("separator")).toBeNull();
  });

  it("fullscreen_widens_over_the_board_then_covers_it", async () => {
    const onCover = vi.fn();
    render(<Panel boardWidth={1222} onCover={onCover} />);
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    expect(aside()).toHaveAttribute("data-layout", "full");
    expect(aside()).toHaveAttribute("data-resize", "widen");
    expect(onCover).not.toHaveBeenLastCalledWith(true);
    end("fui-widen");
    expect(aside()).not.toHaveAttribute("data-resize");
    expect(onCover).toHaveBeenLastCalledWith(true);
  });

  it("back_to_the_board_narrows_then_sits_beside_it", async () => {
    const onCover = vi.fn();
    render(<Panel boardWidth={1222} onCover={onCover} />);
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    end("fui-widen");
    await userEvent.click(screen.getByRole("button", { name: "Back to the board" }));
    // The board shows again at once; the panel keeps covering what it covers until it has narrowed.
    expect(onCover).toHaveBeenLastCalledWith(false);
    expect(aside()).toHaveAttribute("data-layout", "full");
    expect(aside()).toHaveAttribute("data-resize", "narrow");
    end("fui-narrow");
    expect(aside()).toHaveAttribute("data-layout", "side");
    expect(aside()).not.toHaveAttribute("data-resize");
  });

  it("with_motion_off_it_changes_at_once", async () => {
    document.documentElement.dataset.animations = "off";
    try {
      const onCover = vi.fn();
      render(<Panel boardWidth={1222} onCover={onCover} />);
      await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
      expect(aside()).toHaveAttribute("data-layout", "full");
      expect(aside()).not.toHaveAttribute("data-resize");
      expect(onCover).toHaveBeenLastCalledWith(true);
    } finally {
      delete document.documentElement.dataset.animations;
    }
  });

  it("a_wide_tab_takes_the_section_and_back_sets_it_aside", async () => {
    function Wide() {
      const panel = usePanel();
      return (
        <>
          <button type="button" onClick={() => panel.setWide(true)}>
            Diff
          </button>
          <DetailPanel agent={agent()} panel={panel} boardWidth={1222} daemonHost="127.0.0.1" onTerminal={null} onClose={() => {}}>
            <p>pane</p>
          </DetailPanel>
        </>
      );
    }
    render(<Wide />);
    await userEvent.click(screen.getByRole("button", { name: "Diff" }));
    expect(aside()).toHaveAttribute("data-layout", "full");
    end("fui-widen");
    await userEvent.click(screen.getByRole("button", { name: "Back to the board" }));
    end("fui-narrow");
    expect(aside()).toHaveAttribute("data-layout", "side");
  });

  it("escape_leaves_a_chosen_fullscreen", async () => {
    render(<Panel boardWidth={1200} />);
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    expect(aside()).toHaveAttribute("data-layout", "full");
    end("fui-widen");
    await userEvent.keyboard("{Escape}");
    end("fui-narrow");
    expect(aside()).toHaveAttribute("data-layout", "side");
  });

  it("a_narrow_window_forces_fullscreen", async () => {
    const { rerender } = render(<Panel boardWidth={1222} />);
    expect(aside()).toHaveAttribute("data-layout", "side");
    rerender(<Panel boardWidth={500} />);
    end("fui-widen");
    expect(aside()).toHaveAttribute("data-layout", "full");
    const back = screen.getByRole("button", { name: "Back to the board" });
    expect(back).toBeDisabled();
    expect(back).toHaveAttribute("title", "the window is too narrow for the board");
    await userEvent.keyboard("{Escape}");
    expect(aside()).toHaveAttribute("data-layout", "full");
    rerender(<Panel boardWidth={1222} />);
    end("fui-narrow");
    expect(aside()).toHaveAttribute("data-layout", "side");
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
});
