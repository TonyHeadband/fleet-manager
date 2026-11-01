import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { Board } from "./Board";
import { FakeEventSource, agent, factory, fakeClient, persona, product, schema } from "../test/fixtures";

// With the detail panel beside it, the board's quiet column (Stopped) gives its place to the panel: it becomes a rail
// of its bar, count and title, and the other columns keep their width.

const agents = [agent({ id: "s1", goal: "old job", status: "stopped" }), agent({ id: "w1", goal: "live job", status: "working" })];
const draw = (props: { rail?: boolean; selected?: string | null }) =>
  render(<Board schema={schema} agents={agents} now={1300} onRun={() => {}} onOpen={() => {}} {...props} />);
const column = (name: string) => screen.getByRole("region", { name });

describe("the quiet column's rail", () => {
  it("the_quiet_column_becomes_a_rail_beside_the_panel", () => {
    draw({ rail: true, selected: "w1" });
    expect(column("Stopped")).toHaveAttribute("data-rail");
    expect(within(column("Stopped")).queryByText("old job")).toBeNull();
    expect(within(column("Stopped")).getByLabelText("1")).toHaveTextContent("01");
    expect(column("Working")).not.toHaveAttribute("data-rail");
    expect(within(column("Working")).getByText("live job")).toBeInTheDocument();
  });

  it("a_selected_card_in_it_keeps_it_whole", () => {
    draw({ rail: true, selected: "s1" });
    expect(column("Stopped")).not.toHaveAttribute("data-rail");
    expect(within(column("Stopped")).getByText("old job")).toBeInTheDocument();
  });

  it("no_panel_no_rail", () => {
    draw({ selected: null });
    expect(column("Stopped")).not.toHaveAttribute("data-rail");
  });

  it("the_columns_share_the_board_and_the_rail_keeps_its_width", () => {
    const css = readFileSync("src/board/board.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.board \{[^}]*display: flex;/);
    expect(css).toMatch(/\.column \{[^}]*flex: 1 1 0;/);
    expect(css).toMatch(/\.column\[data-rail\] \{[^}]*flex: 0 0 var\(--layout-rail\);/);
    const motion = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    // The rail closes with the panel's push and opens with its pull.
    expect(motion).toMatch(/\.column \{ transition: flex-grow var\(--motion-pull\) var\(--ease-pull\), flex-basis var\(--motion-pull\) var\(--ease-pull\); \}/);
    expect(motion).toMatch(/\.column\[data-rail\] \{ transition: flex-grow var\(--motion-push\) var\(--ease-push\), flex-basis var\(--motion-push\) var\(--ease-push\); \}/);
  });
});

describe("the board and the panel", () => {
  beforeEach(() => {
    window.location.hash = "";
    localStorage.clear();
    FakeEventSource.reset();
  });

  it("opening_a_card_rails_stopped_and_the_diff_takes_the_section", async () => {
    document.documentElement.dataset.animations = "off";
    try {
      const client = fakeClient({
        personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }),
        products: vi.fn().mockResolvedValue({ products: [product("p1", "Pie")] }),
        diff: vi.fn().mockResolvedValue({ workspaces: [] }),
      });
      render(<App client={client} eventSource={factory} nowSeconds={() => 1300} />);
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
      await userEvent.click(await screen.findByText("live job"));
      expect(column("Stopped")).toHaveAttribute("data-rail");
      const detail = screen.getByRole("complementary", { name: "Detail" });
      expect(detail).toHaveAttribute("data-layout", "side");
      await userEvent.click(within(detail).getByRole("tab", { name: "Diff" }));
      expect(detail).toHaveAttribute("data-layout", "full");
      await userEvent.click(within(detail).getByRole("tab", { name: "Chat" }));
      expect(detail).toHaveAttribute("data-layout", "side");
      await userEvent.click(within(detail).getByRole("button", { name: "Close" }));
      expect(column("Stopped")).not.toHaveAttribute("data-rail");
    } finally {
      delete document.documentElement.dataset.animations;
    }
  });
});
