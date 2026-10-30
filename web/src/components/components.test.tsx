import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Board } from "./Board";
import { Card, type CardKind } from "./Card";
import { groups } from "../board/groups";
import type { Agent } from "../api/generated/Agent";
import { PreviewPane } from "./PreviewPane";
import { ReplyBox } from "./ReplyBox";
import { useState } from "react";
import { readFileSync } from "node:fs";
import { RepoFilter } from "./RepoFilter";
import { PersonaFilter } from "./PersonaFilter";
import { ReleaseFilter } from "./ReleaseFilter";
import { FormsPage } from "./FormsPage";
import { FleetClient } from "../api/client";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, agent, agentIn, persona, schema, workspace } from "../test/fixtures";
import { title } from "../api/schema";
import { personaRows, personaTags } from "../board/filters";

const NOW = 1000 + 5 * 60;
// An orchestrated task "Fix login" (o) and its fleet agents.
const o = (over: Partial<Agent> = {}) => agent({ id: "o", kind: "orchestrator", goal: "Fix login", status: "working", ...over });
const f = (id: string, over: Partial<Agent> = {}) => agent({ id, kind: "specialist", task: "o", role: "rust", goal: `step ${id}`, status: "working", ...over });
beforeEach(() => FakeEventSource.reset());

describe("Board", () => {
  it("Board_renders_columns_from_schema_only", () => {
    const custom: UiSchema = {
      ...schema,
      columns: [
        { id: "left", title: "Alpha lane", statuses: ["working"], empty_hint: "nothing left", tone: "active", dim: false },
        { id: "right", title: "Beta lane", statuses: ["review"], empty_hint: "nothing right", tone: "done", dim: false },
      ],
    };
    render(<Board schema={custom} agents={[agent({ id: "w", status: "working" })]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual(["Alpha lane", "Beta lane"]);
    expect(within(screen.getByRole("region", { name: "Alpha lane" })).getByText("fix the tests")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Beta lane" })).getByText("nothing right")).toBeInTheDocument();
    expect(screen.queryByText("Needs you")).not.toBeInTheDocument();
  });

  it("puts each agent in the column the schema maps its status to", () => {
    const agents = [agent({ id: "q", status: "queued", goal: "t-q" }), agent({ id: "d", status: "stopped", goal: "t-d" }), agent({ id: "n", status: "needs_you", goal: "t-n" })];
    render(<Board schema={schema} agents={agents} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(within(screen.getByRole("region", { name: "Queued" })).getByText("t-q")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Stopped" })).getByText("t-d")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Needs you" })).getByText("t-n")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Talking" })).not.toBeInTheDocument();
  });

  it("lists the longest-waiting card first", () => {
    const agents = [agent({ id: "new", goal: "newer", status_since: 900 }), agent({ id: "old", goal: "older", status_since: 100 })];
    render(<Board schema={schema} agents={agents} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const titles = within(screen.getByRole("region", { name: "Working" })).getAllByRole("article").map((c) => c.textContent);
    expect(titles[0]).toContain("older");
  });
});

describe("Column", () => {
  // Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
  const boardCss = readFileSync("src/board/board.css", "utf8");
  const header = (name: string) => screen.getByRole("region", { name }).querySelector(".column-header")!;

  it("column_titles_are_mono_uppercase_and_keep_their_labels", () => {
    render(<Board schema={schema} agents={[]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const title = within(screen.getByRole("region", { name: "Needs you" })).getByText("Needs you");
    expect(title).toHaveClass("mono-label");
    expect(boardCss).toMatch(/\.mono-label \{[^}]*text-transform: uppercase/);
  });

  it("the_count_is_two_digits_and_read_as_a_number", () => {
    render(<Board schema={schema} agents={[agent({ id: "x" }), agent({ id: "y" })]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const count = header("Working").querySelector(".count")!;
    expect(count).toHaveTextContent("02");
    expect(count).toHaveAttribute("aria-label", "2");
    expect(count).toHaveAttribute("data-tone", "active");
  });

  it("column_headers_carry_ticks", () => {
    render(<Board schema={schema} agents={[]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    for (const region of screen.getAllByRole("region")) expect(region.querySelector(".column-header .ticks")).not.toBeNull();
    expect(boardCss).toMatch(/\.ticks \{[^}]*var\(--column-tick-step\)[^}]*var\(--column-tick-len\)/);
  });

  it("the_stopped_column_is_darker", () => {
    render(<Board schema={schema} agents={[]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.getByRole("region", { name: "Stopped" })).toHaveAttribute("data-dim");
    expect(screen.getByRole("region", { name: "Queued" })).not.toHaveAttribute("data-dim");
    expect(boardCss).toMatch(/\.column\[data-dim\] \.column-body \{[^}]*var\(--sunken\)/);
  });
});

describe("Card", () => {
  // Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
  const needs = agent({ status: "needs_you", status_since: 1000, blocked_reason: "permission", activity: "wants to run Bash", paused: false });
  const draw = (a = agent(), over: Partial<Parameters<typeof Card>[0]> = {}) => {
    const onRun = vi.fn();
    const onOpen = vi.fn();
    render(<Card schema={schema} agent={a} now={NOW} onRun={onRun} onOpen={onOpen} {...over} />);
    return { onRun, onOpen, card: screen.getByRole("article") };
  };

  it("card_shows_task_goal_activity_and_time_waiting_for_needs_you", () => {
    const { card } = draw(needs);
    for (const t of ["core", "fix the tests", "wants to run Bash", "Permission needed", "5m"]) expect(within(card).getByText(t)).toBeInTheDocument();
    expect(card).toHaveAttribute("data-tone", "attention");
  });

  it("needs_you_card_has_no_answer_field", () => {
    const { card } = draw(needs);
    expect(within(card).queryByRole("textbox")).toBeNull();
    expect(within(card).queryByRole("button", { name: "Answer" })).toBeNull();
  });

  it("a stopped card shows its cause, tinted by the schema, and its size", () => {
    const { card } = draw(agent({ status: "stopped", stop_cause: "crashed", disk_bytes: 7_600_000_000, activity: "crashed while running Bash" }));
    expect(within(card).getByText("Crashed")).toHaveAttribute("data-tone", "danger");
    expect(within(card).getByText("7.6 GB")).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });

  it("the_card_names_the_first_repo_and_counts_the_others", () => {
    const { card } = draw(agent({ workspaces: [workspace("core"), workspace("portal")] }));
    expect(within(card).getByText("core")).toHaveAttribute("title", "core\nportal");
    expect(within(card).getByText("+1")).toHaveAttribute("title", "core\nportal");
  });

  it("has_changes_is_a_dot_not_a_badge", () => {
    draw(agent({ status: "review", has_changes: true }));
    expect(screen.getByRole("img", { name: "Has changes" })).toHaveAttribute("title", "Has changes");
    expect(screen.queryByText("Has changes")).toBeNull();
  });

  it("shows only the fields whose condition holds", () => {
    draw(agent({ status: "working", paused: false }));
    expect(screen.queryByTitle("Blocked")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Paused")).not.toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "Has changes" })).toBeNull();
  });

  it("clicking_the_card_body_returns_open", async () => {
    const { onOpen } = draw();
    await userEvent.click(screen.getByText("running Bash"));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }));
  });

  it("a_card_opens_from_the_keyboard_and_its_actions_keep_their_keys", async () => {
    const { onOpen, onRun } = draw();
    await userEvent.tab();
    expect(screen.getByRole("article")).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledTimes(1);
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "Pause" })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: "pause" }), expect.anything(), undefined);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("clicking_stop_does_not_open_the_panel", async () => {
    const { onOpen, onRun } = draw();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onOpen).not.toHaveBeenCalled();
    expect(onRun, "the first click only asks for confirmation").not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Click again to stop" }));
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: "stop" }), expect.anything(), undefined);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("a_stopped_card_offers_details_and_a_running_one_does_not", async () => {
    window.location.hash = "";
    draw(agent({ status: "stopped", stop_cause: "finished" }));
    await userEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(window.location.hash).toBe("#tasks?task=a1");
    cleanup();
    draw(agent({ status: "working" }));
    expect(screen.queryByRole("button", { name: "Details" })).toBeNull();
  });

  it("a_paused_agent_offers_resume", () => {
    draw(agent({ status: "working", paused: true }));
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
  });

  it("an_action_without_an_icon_is_a_text_button", async () => {
    const stop = schema.actions.find((a) => a.id === "stop")!;
    const custom: UiSchema = { ...schema, actions: [...schema.actions, { ...stop, id: "archive", label: "Archive" }] };
    const { onRun } = draw(agent(), { schema: custom });
    await userEvent.click(screen.getByRole("button", { name: "Archive" }));
    expect(onRun).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Confirm Archive" }));
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: "archive" }), expect.anything(), undefined);
  });

  it("icon_buttons_show_their_action_as_a_tooltip", async () => {
    draw(agent({ status: "working" }));
    expect(screen.getByRole("button", { name: "Pause" })).toHaveAttribute("title", "Pause");
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(screen.getByRole("button", { name: "Click again to stop" })).toHaveAttribute("title", "Click again to stop");
  });

  it("hovering_the_title_shows_the_whole_goal", () => {
    const goal = "Fix the flaky tests in the payments service. Then look at the retry logic too.";
    draw(agent({ goal }));
    expect(screen.getByText(title(agent({ goal })))).toHaveAttribute("title", goal);
  });

  it("a_selected_card_fades_and_carries_brackets", () => {
    const { card } = draw(agent(), { selected: true });
    expect(card).toHaveAttribute("data-selected");
    expect(card.querySelectorAll(".bracket")).toHaveLength(4);
    cleanup();
    const plain = draw(agent()).card;
    expect(plain).not.toHaveAttribute("data-selected");
    expect(plain.querySelectorAll(".bracket")).toHaveLength(0);
  });

  it("the_persona_tag_appears_only_with_two_personas", () => {
    const people = [persona("ana"), persona("bo")];
    const one = [agent({ persona: "ana" }), agent({ id: "a2", persona: "ana" })];
    expect(personaTags(one, personaRows(people, one))).toEqual({});
    const three = [agent({ persona: "ana" }), agent({ id: "a2", persona: "gone" }), agent({ id: "a3", persona: "" })];
    expect(personaTags(three, personaRows(people, three))).toEqual({
      ana: "ana",
      gone: "gone",
      "": "no persona",
    });
    const { card } = draw(agent(), { tag: "Ana" });
    expect(within(card).getByText("Ana")).toHaveAttribute("title", "Ana");
  });
});

describe("Board by tasks", () => {
  // Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
  const board = (agents: Agent[], over: Partial<Parameters<typeof Board>[0]> = {}) => {
    const props = { schema, agents, now: NOW, onRun: vi.fn(), onOpen: vi.fn(), onStopTask: vi.fn(), onRestart: vi.fn(), ...over };
    const view = render(<Board {...props} />);
    return { ...props, rerender: (next: Agent[], more: Partial<Parameters<typeof Board>[0]> = {}) => view.rerender(<Board {...props} agents={next} {...more} />) };
  };
  const rows = () => screen.queryAllByRole("button", { name: /, (working|needs you|queued|review|stopped)$/ }).map((r) => r.getAttribute("aria-label"));

  it("stuck_row_first_and_group_forced_open", async () => {
    const working = [o(), f("f1", { status_since: 900, goal: "write code" }), f("f2", { status_since: 500, goal: "drop cookie" })];
    const { onOpen, rerender } = board(working);
    expect(rows(), "longest in status first").toEqual(["rust drop cookie, working", "rust write code, working"]);
    await userEvent.click(screen.getByRole("button", { name: "▾ 2 fleet" }));
    expect(screen.getByRole("button", { name: "▸ 2 fleet" })).toBeInTheDocument();
    expect(rows()).toEqual([]);
    rerender([o(), f("f1", { status_since: 900, goal: "write code", status: "needs_you", blocked_reason: "question" }), f("f2", { status_since: 500, goal: "drop cookie" })]);
    expect(rows(), "a stuck agent opens its task and comes first").toEqual(["rust write code, needs you", "rust drop cookie, working"]);
    await userEvent.click(screen.getByRole("button", { name: "rust drop cookie, working" }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "f2" }));
  });

  it("display_switch_modes", () => {
    const agents = [o(), f("f1"), f("f2", { status: "review" })];
    const { rerender } = board(agents);
    expect(rows()).toHaveLength(2);
    expect(screen.queryByText("↳ Fix login")).toBeNull();
    rerender(agents, { display: "show" });
    expect(screen.getAllByText("↳ Fix login")).toHaveLength(2);
    expect(rows()).toEqual([]);
    rerender(agents, { display: "hide" });
    expect(screen.queryAllByText("↳ Fix login")).toEqual([]);
    expect(rows()).toEqual([]);
    expect(screen.queryByRole("button", { name: /fleet$/ })).toBeNull();
    expect(screen.getByRole("img", { name: "1 Working" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "1 Review" })).toBeInTheDocument();
  });

  it("a_fleet_row_carries_the_status_edge_and_no_dot", () => {
    board([o(), f("f1", { goal: "drop cookie", status: "needs_you", blocked_reason: "permission" })]);
    const row = screen.getByRole("button", { name: "rust drop cookie, needs you" });
    expect(row).toHaveAttribute("data-tone", "attention");
    expect(row).toHaveAttribute("data-outline", "stuck");
    expect(row.querySelector(".dot")).toBeNull();
    expect(within(row).getByRole("img", { name: "Needs you" })).toBeInTheDocument();
  });

  it("a_fleet_agent_goes_by_its_personas_nickname", () => {
    const personas = [{ ...persona("me"), nicknames: { rust: "Ferris" } }];
    board([o({ persona: "me" }), f("f1", { persona: "me", goal: "drop cookie" }), f("f2", { persona: "me", role: "docs", goal: "write docs" })], { personas });
    expect(screen.getByRole("button", { name: "Ferris (rust) drop cookie, working" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "docs write docs, working" })).toBeInTheDocument();
  });

  it("a_selected_fleet_row_is_outlined_in_the_accent", () => {
    board([o(), f("f1", { goal: "drop cookie" })], { selected: "f1" });
    const row = screen.getByRole("button", { name: "rust drop cookie, working" });
    expect(row).toHaveAttribute("data-selected");
    expect(row).toHaveAttribute("data-outline", "selected");
  });

  it("clicking_a_compact_row_anywhere_opens_the_panel", async () => {
    const { onOpen } = board([o(), f("f1", { goal: "drop cookie" })]);
    const row = screen.getByRole("button", { name: "rust drop cookie, working" });
    await userEvent.click(row);
    await userEvent.click(within(row).getByText("drop cookie"));
    expect(onOpen).toHaveBeenCalledTimes(2);
    row.focus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledTimes(3);
  });

  it("column_counts_count_tasks", () => {
    board([o(), f("f1"), f("f2")]);
    expect(screen.getByRole("region", { name: "Working" }).querySelector(".count")).toHaveTextContent("01");
  });

  it("the_fold_survives_a_move_between_columns", async () => {
    const { rerender } = board([o(), f("f1"), f("f2")]);
    await userEvent.click(within(screen.getByRole("region", { name: "Working" })).getByRole("button", { name: "▾ 2 fleet" }));
    rerender([o({ status: "needs_you", blocked_reason: "question" }), f("f1"), f("f2")]);
    expect(within(screen.getByRole("region", { name: "Needs you" })).getByRole("button", { name: "▸ 2 fleet" })).toBeInTheDocument();
    rerender([o(), f("f1", { status: "needs_you", blocked_reason: "question" }), f("f2")]);
    expect(screen.getByRole("button", { name: "▾ 2 fleet" })).toBeInTheDocument();
  });

  it("a_compact_row_marks_an_open_pr", () => {
    // As Lite's column.rs: until the fleet view is loaded, the lead's activity says the task has a PR open.
    const { rerender } = board([o({ activity: "PR open" }), f("f1", { goal: "drop cookie" }), f("f2", { goal: "write code" })]);
    for (const name of ["rust drop cookie, working", "rust write code, working"]) {
      expect(within(screen.getByRole("button", { name })).getByRole("img", { name: "PR open" })).toBeInTheDocument();
    }
    rerender([o(), f("f1", { goal: "drop cookie", activity: "PR open" })]);
    expect(within(screen.getByRole("button", { name: "rust drop cookie, working" })).queryByRole("img", { name: "PR open" })).toBeNull();
  });
});

describe("Task cards", () => {
  // Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
  const lead = (agents: Agent[], over: Partial<Extract<CardKind, { type: "lead" }>> = {}) => {
    const g = groups(agents)[0]!;
    const props = { onFold: vi.fn(), onStopTask: vi.fn(), onRestart: vi.fn() };
    render(<Card schema={schema} agent={g.lead} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} kind={{ type: "lead", group: g, display: "compact", open: true, ...props, ...over }} />);
    return props;
  };

  it("group_sits_in_rolled_up_column", () => {
    lead([o(), f("f1"), f("f2", { status: "needs_you", blocked_reason: "permission" })]);
    const card = screen.getByRole("group", { name: "Fix login, 2 fleet agents" });
    expect(card).toHaveAttribute("data-tone", "attention");
    expect(within(card).getByRole("img", { name: "Orchestrator" })).toBeInTheDocument();
  });

  it("the_stuck_agent_names_itself_in_the_badge", () => {
    lead([o(), f("f1"), f("f2", { status: "needs_you", blocked_reason: "permission", goal: "drop cookie" })]);
    expect(screen.getByText("drop cookie · Permission needed")).toHaveAttribute("title", "Task");
    expect(screen.getAllByRole("img", { name: "Specialist" })).toHaveLength(1);
  });

  it("a_stopped_orchestrator_with_live_fleet_asks_for_a_restart", () => {
    lead([o({ status: "stopped", stop_cause: "crashed" }), f("f1")]);
    expect(screen.getByText("Orchestrator stopped · Restart?")).toBeInTheDocument();
    expect(screen.queryByText("Crashed")).toBeNull();
  });

  it("an_ended_orchestrator_without_fleet_asks_for_a_restart", async () => {
    const { onRestart } = lead([o({ status: "stopped", stop_cause: "exited" }), f("f1", { status: "stopped", stop_cause: "finished" })]);
    expect(screen.getByText("Orchestrator stopped · Restart?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restart orchestrator" }));
    expect(onRestart).toHaveBeenCalledWith("o");
  });

  it("a_stopped_task_stays_stopped_and_offers_a_restart", () => {
    lead([o({ status: "stopped", stop_cause: "stopped" }), f("f1", { status: "stopped", stop_cause: "stopped" })]);
    expect(screen.queryByText("Orchestrator stopped · Restart?")).toBeNull();
    expect(screen.getByRole("button", { name: "Restart orchestrator" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" }), "nothing left to stop").toBeNull();
  });

  it("task_stop_is_one_click_with_the_live_count", async () => {
    const { onStopTask } = lead([o(), f("f1"), f("f2"), f("f3", { status: "stopped" })]);
    expect(screen.getAllByRole("button", { name: "Stop" }), "the agent's own Stop gives way to the task's").toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Restart orchestrator" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStopTask).toHaveBeenCalledWith("o", 2);
  });

  it("the_fold_toggle_reads_its_state", async () => {
    const { onFold } = lead([o(), f("f1"), f("f2")]);
    expect(screen.getByRole("img", { name: "2 Working" })).toHaveAttribute("title", "Working");
    await userEvent.click(screen.getByRole("button", { name: "▾ 2 fleet" }));
    expect(onFold).toHaveBeenCalledTimes(1);
    cleanup();
    lead([o(), f("f1"), f("f2")], { open: false });
    expect(screen.getByRole("button", { name: "▸ 2 fleet" })).toBeInTheDocument();
    cleanup();
    lead([o(), f("f1"), f("f2")], { display: "hide" });
    expect(screen.queryByRole("button", { name: /fleet$/ })).toBeNull();
    expect(screen.getByRole("img", { name: "2 Working" })).toBeInTheDocument();
  });

  it("a_full_fleet_card_names_its_task_and_is_outlined", () => {
    render(<Card schema={schema} agent={f("f2", { status: "needs_you", blocked_reason: "question" })} now={NOW} tag="Ferris (rust)" onRun={vi.fn()} onOpen={vi.fn()} kind={{ type: "fleet", task: "Fix login", stuck: true }} />);
    const card = screen.getByRole("article");
    expect(within(card).getByText("↳ Fix login")).toHaveAttribute("title", "↳ Fix login");
    expect(within(card).getByRole("img", { name: "Specialist" })).toBeInTheDocument();
    expect(within(card).getByText("Ferris (rust)")).toBeInTheDocument();
    expect(card).toHaveAttribute("data-outline", "stuck");
  });

  it("solo_cards_unchanged", () => {
    render(<Card schema={schema} agent={agent()} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.getByRole("article").querySelectorAll(".card-row")).toHaveLength(4);
    expect(screen.getByRole("article")).not.toHaveAttribute("data-outline");
    cleanup();
    lead([o(), f("f1")]);
    expect(screen.getByRole("group").querySelectorAll(".card-row")).toHaveLength(5);
    cleanup();
    render(<Card schema={schema} agent={f("f1")} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} kind={{ type: "fleet", task: "Fix login", stuck: false }} />);
    expect(screen.getByRole("article").querySelectorAll(".card-row")).toHaveLength(5);
  });
});

describe("ReplyBox", () => {
  // Ported from crates/fleet-gui/src/components/reply.rs.
  const Box = ({ onSend = vi.fn(), initial = "" }: { onSend?: (t: string) => void; initial?: string }) => {
    const [text, setText] = useState(initial);
    return <ReplyBox label="Answer" text={text} onText={setText} onSend={onSend} />;
  };

  it("sends_the_typed_text_and_keeps_literal_characters", async () => {
    const onSend = vi.fn();
    render(<Box onSend={onSend} />);
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(screen.getByText("Ctrl+Enter")).toBeInTheDocument();
    await userEvent.click(screen.getByPlaceholderText("Answer…"));
    await userEvent.paste('$(x) "q" {a}'); // paste, not type: userEvent.type reads `{` as key syntax
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(onSend).toHaveBeenCalledWith('$(x) "q" {a}');
  });

  it("enter_is_a_newline_and_a_blank_reply_is_not_sent", async () => {
    const onSend = vi.fn();
    render(<Box onSend={onSend} initial="   " />);
    await userEvent.type(screen.getByRole("textbox"), "{Control>}{Enter}{/Control}");
    expect(onSend).not.toHaveBeenCalled();
    await userEvent.type(screen.getByRole("textbox"), "a{Enter}b");
    expect(screen.getByRole("textbox")).toHaveValue("   a\nb");
    expect(onSend).not.toHaveBeenCalled();
  });
});

describe("Header filters", () => {
  // Ported from crates/fleet-gui/tests/panels.rs and app.rs; the names are Lite's.
  const Repos = ({ onChange = vi.fn(), initial = [] as string[] }) => {
    const [selected, setSelected] = useState(initial);
    return (
      <RepoFilter
        repos={[["core", 2], ["portal", 1], ["infra", 0]]}
        selected={selected}
        onChange={(next) => {
          setSelected(next);
          onChange(next);
        }}
      />
    );
  };
  const filter = (name: string) => screen.getByRole("button", { name });

  it("filter_summary_names_the_selection", async () => {
    render(<Repos />);
    expect(filter("Repo filter")).toHaveTextContent("Repo All repos");
    await userEvent.click(filter("Repo filter"));
    await userEvent.click(screen.getByRole("checkbox", { name: "core" }));
    expect(filter("Repo filter")).toHaveTextContent("Repo core");
    await userEvent.click(screen.getByRole("checkbox", { name: "portal" }));
    expect(filter("Repo filter")).toHaveTextContent("Repo 2 repos");
  });

  it("the_repo_filter_reports_its_value_as_written", () => {
    render(<Repos />);
    expect(filter("Repo filter")).toHaveAttribute("aria-haspopup");
    expect(filter("Repo filter").textContent?.replace(/\s+/g, " ").trim()).toBe("Repo All repos");
  });

  it("repo_filter_toggles_repos_in_click_order", async () => {
    const onChange = vi.fn();
    render(<Repos onChange={onChange} />);
    await userEvent.click(filter("Repo filter"));
    await userEvent.click(screen.getByRole("checkbox", { name: "portal" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "core" }));
    expect(onChange).toHaveBeenLastCalledWith(["portal", "core"]);
    expect(screen.getByRole("checkbox", { name: "core" }), "the menu stays open for the next pick").toBeChecked();
    await userEvent.click(screen.getByRole("checkbox", { name: "portal" }));
    expect(onChange).toHaveBeenLastCalledWith(["core"]);
    expect(onChange, "one change per click").toHaveBeenCalledTimes(3);
  });

  it("all_repos_clears_the_selection", async () => {
    const onChange = vi.fn();
    render(<Repos onChange={onChange} initial={["core", "portal"]} />);
    await userEvent.click(filter("Repo filter"));
    expect(screen.getByRole("checkbox", { name: "All repos" })).not.toBeChecked();
    await userEvent.click(screen.getByRole("checkbox", { name: "All repos" }));
    expect(onChange).toHaveBeenCalledWith([]);
    await userEvent.click(screen.getByRole("checkbox", { name: "All repos" }));
    expect(onChange, "unticking it has nothing to fall back on").toHaveBeenCalledTimes(1);
  });

  it("each_repo_shows_its_count", async () => {
    render(<Repos />);
    await userEvent.click(filter("Repo filter"));
    expect(screen.getByRole("checkbox", { name: "core" }).closest(".menu-row")).toHaveTextContent("core2");
  });

  it("the_menu_closes_on_a_click_outside_or_escape", async () => {
    render(<Repos />);
    await userEvent.click(filter("Repo filter"));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("checkbox")).toBeNull();
    await userEvent.click(filter("Repo filter"));
    await userEvent.click(document.body);
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("the_persona_filter_names_its_selection", async () => {
    const onChange = vi.fn();
    const rows = [{ id: "p", label: "Ann (p)", count: 1 }, { id: "w", label: "Ann (w)", count: 2 }];
    const { rerender } = render(<PersonaFilter rows={rows} selected={[]} onChange={onChange} />);
    expect(filter("Persona filter")).toHaveTextContent("Persona All personas");
    await userEvent.click(filter("Persona filter"));
    await userEvent.click(screen.getByRole("checkbox", { name: "Ann (w)" }));
    expect(onChange, "the filter keeps the id").toHaveBeenCalledWith(["w"]);
    rerender(<PersonaFilter rows={rows} selected={["w"]} onChange={onChange} />);
    expect(filter("Persona filter")).toHaveTextContent("Persona Ann (w)");
    rerender(<PersonaFilter rows={rows} selected={["w", "p"]} onChange={onChange} />);
    expect(filter("Persona filter")).toHaveTextContent("Persona 2 personas");
  });

  it("the_release_filter_picks_one_and_closes", async () => {
    const onChange = vi.fn();
    const releases = [{ id: 1, product: "p1", version: "1.0", start: "", target: "", released_at: null }];
    render(<ReleaseFilter releases={releases} value={{ type: "all" }} onChange={onChange} />);
    expect(filter("Release filter")).toHaveTextContent("Release All releases");
    await userEvent.click(filter("Release filter"));
    expect(screen.getAllByRole("button", { name: /All releases|1\.0|Backlog/ }).map((b) => b.textContent)).toEqual(["All releases", "1.0", "Backlog"]);
    await userEvent.click(screen.getByRole("button", { name: "All releases", pressed: true }));
    expect(onChange, "the row already picked does nothing").not.toHaveBeenCalled();
    await userEvent.click(filter("Release filter"));
    await userEvent.click(screen.getByRole("button", { name: "Backlog" }));
    expect(onChange).toHaveBeenCalledWith({ type: "backlog" });
    expect(screen.queryByRole("button", { name: "Backlog" })).toBeNull();
  });
});

describe("PreviewPane diff tab", () => {
  const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;
  const report = {
    workspaces: [
      { repo: "core", diff: "+++ b/new.txt\n+hello\n", truncated: false },
      { repo: "portal", diff: "", truncated: false },
      { repo: "big", diff: "+cut here", truncated: true },
    ],
  };

  it("PreviewPane_diff_tab_shows_each_workspace_diff", async () => {
    const loadDiff = vi.fn().mockResolvedValue(report);
    render(<PreviewPane baseUrl="" agentId="a1" title="t" factory={factory} loadDiff={loadDiff} />);
    expect(screen.getByRole("tab", { name: "Terminal" })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    expect(await screen.findByText("+hello", { exact: false })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "core" })).toBeInTheDocument();
    expect(screen.getByText("No changes")).toBeInTheDocument();
    expect(screen.getByText(/cut at the size limit/)).toBeInTheDocument();
    expect(loadDiff).toHaveBeenCalledTimes(1);
  });

  it("reloads on Refresh and shows a failure instead of an empty diff", async () => {
    const loadDiff = vi.fn().mockRejectedValueOnce(new Error("no such agent")).mockResolvedValue(report);
    render(<PreviewPane baseUrl="" agentId="a1" title="t" factory={factory} loadDiff={loadDiff} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("no such agent");
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("+hello", { exact: false })).toBeInTheDocument();
    expect(loadDiff).toHaveBeenCalledTimes(2);
  });

  it("stays read-only on both tabs", async () => {
    const { container } = render(<PreviewPane baseUrl="" agentId="a1" title="t" factory={factory} loadDiff={vi.fn().mockResolvedValue(report)} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    await screen.findByText("+hello", { exact: false });
    expect(container.querySelector("input, textarea, [contenteditable='true'], select")).toBeNull();
  });

  it("offers no Diff tab when the caller cannot load one", () => {
    render(<PreviewPane baseUrl="" agentId="a1" title="t" factory={factory} />);
    expect(screen.queryByRole("tab", { name: "Diff" })).not.toBeInTheDocument();
  });
});

describe("PreviewPane as Lite draws it", () => {
  // Ported from crates/fleet-gui/tests/panels.rs; the names are Lite's.
  const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;
  const css = readFileSync("src/panel/panel.css", "utf8");

  it("a_diff_that_lands_after_another_agent_opened_is_dropped", async () => {
    const answers: Array<(r: { workspaces: { repo: string; diff: string; truncated: boolean }[] }) => void> = [];
    const loadDiff = vi.fn(() => new Promise<never>((resolve) => answers.push(resolve as never)));
    const report = (repo: string) => ({ workspaces: [{ repo, diff: `+ ${repo} change`, truncated: false }] });
    const { rerender } = render(<PreviewPane baseUrl="" agentId="a" factory={factory} loadDiff={loadDiff} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    rerender(<PreviewPane baseUrl="" agentId="b" factory={factory} loadDiff={loadDiff} />);
    await act(async () => answers[0]!(report("from-a")));
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    expect(loadDiff, "b's own diff is asked for").toHaveBeenCalledTimes(2);
    expect(screen.queryByText("+ from-a change")).toBeNull();
    // Two Refreshes: only the latest answer shows, whichever lands last.
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await act(async () => answers[2]!(report("newest")));
    await act(async () => answers[1]!(report("older")));
    expect(screen.getByText("+ newest change")).toBeInTheDocument();
    expect(screen.queryByText("+ older change")).toBeNull();
  });

  it("another_agent_never_draws_the_previous_agents_tab", async () => {
    const drawn: string[] = [];
    const Probe = ({ id }: { id: string }) => {
      drawn.push(id);
      return <p>{`${id}'s plan`}</p>;
    };
    const tabs = (id: string) => [{ id: "breakdown", label: "Breakdown", first: false, node: <Probe id={id} /> }];
    const { rerender } = render(<PreviewPane baseUrl="" agentId="a" factory={factory} extraTabs={tabs("a")} />);
    await userEvent.click(screen.getByRole("tab", { name: "Breakdown" }));
    rerender(<PreviewPane baseUrl="" agentId="b" factory={factory} extraTabs={tabs("b")} />);
    expect(screen.getByRole("tab", { name: "Terminal" })).toHaveAttribute("aria-selected", "true");
    expect(drawn, "not even one frame of b on a's Breakdown tab").not.toContain("b");
  });

  it("preview_says_session_ended_when_closed", () => {
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} />);
    act(() => FakeEventSource.for("/terminal").emit("pane", "$ ready"));
    act(() => FakeEventSource.for("/terminal").emit("closed", ""));
    const note = screen.getByText("Session ended");
    expect(note.compareDocumentPosition(screen.getByLabelText("Terminal preview")) & Node.DOCUMENT_POSITION_FOLLOWING, "the note sits above the text").toBeTruthy();
  });

  it("a_pane_padded_with_blank_rows_still_shows_its_text", () => {
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} />);
    act(() => FakeEventSource.for("/terminal").emit("pane", "$ ready\n> hello   \n\n\n   \n"));
    expect(screen.getByLabelText("Terminal preview").textContent).toBe("$ ready\n> hello");
  });

  it("long_terminal_lines_keep_one_row_each_instead_of_wrapping_over_their_neighbours", () => {
    expect(css).toMatch(/\.pane \{[^}]*white-space: pre;/);
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} />);
    act(() => FakeEventSource.for("/terminal").emit("pane", "x".repeat(2500)));
    expect(screen.getByLabelText("Terminal preview").textContent, "a line is cut at 2000 characters").toHaveLength(2000);
  });

  it("diff_lines_are_coloured_by_their_prefix", async () => {
    const loadDiff = vi.fn().mockResolvedValue({ workspaces: [{ repo: "core", diff: "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same", truncated: false }] });
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} loadDiff={loadDiff} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    await screen.findByText("+new");
    const kinds = [...screen.getByLabelText("Changes").querySelectorAll("[data-line]")].map((l) => l.getAttribute("data-line"));
    expect(kinds).toEqual(["file", "file", "hunk", "del", "add", "ctx"]);
  });

  it("switching_back_to_the_terminal_keeps_the_loaded_diff", async () => {
    const loadDiff = vi.fn().mockResolvedValue({ workspaces: [{ repo: "core", diff: "+x", truncated: false }] });
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} loadDiff={loadDiff} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    await screen.findByText("+x");
    await userEvent.click(screen.getByRole("tab", { name: "Terminal" }));
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    expect(screen.getByText("+x")).toBeInTheDocument();
    expect(loadDiff).toHaveBeenCalledTimes(1);
  });

  it("the_diff_says_loading_until_it_lands", async () => {
    let land!: (r: { workspaces: [] }) => void;
    render(<PreviewPane baseUrl="" agentId="a1" factory={factory} loadDiff={() => new Promise((r) => (land = r))} />);
    await userEvent.click(screen.getByRole("tab", { name: "Diff" }));
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    await act(async () => land({ workspaces: [] }));
    expect(screen.queryByText("Loading…")).toBeNull();
  });

  it("another_tab_can_come_first_and_be_picked", async () => {
    const onTab = vi.fn();
    const extra = [{ id: "breakdown", label: "Breakdown", first: true, node: <p>the plan</p> }];
    const { rerender } = render(<PreviewPane baseUrl="" agentId="a1" factory={factory} loadDiff={vi.fn()} extraTabs={extra} tab="breakdown" onTab={onTab} />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Breakdown", "Terminal", "Diff"]);
    expect(screen.getByText("the plan")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "Terminal" }));
    expect(onTab).toHaveBeenCalledWith("terminal");
    rerender(<PreviewPane baseUrl="" agentId="a1" factory={factory} loadDiff={vi.fn()} extraTabs={[{ ...extra[0]!, first: false }]} tab="terminal" onTab={onTab} />);
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Terminal", "Diff", "Breakdown"]);
  });
});

describe("PreviewPane", () => {
  it("PreviewPane_renders_text_stream_read_only", () => {
    const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;
    const { container } = render(<PreviewPane baseUrl="" agentId="a1" title="fix the tests" factory={factory} />);
    act(() => FakeEventSource.for("/terminal").emit("pane", "$ ready\n> hello"));
    const pane = screen.getByLabelText("Terminal preview");
    expect(pane.textContent).toBe("$ ready\n> hello");
    expect(container.querySelector("input, textarea, [contenteditable='true'], select")).toBeNull();
  });

  it("says so when the session has ended", () => {
    const factory = (u: string) => new FakeEventSource(u) as unknown as EventSource;
    render(<PreviewPane baseUrl="" agentId="a1" title="t" factory={factory} />);
    act(() => FakeEventSource.for("/terminal").emit("closed", ""));
    expect(screen.getByText("Session ended")).toBeInTheDocument();
  });
});

describe("FormsPage", () => {
  it("creates a release for the picked product from the described form", async () => {
    const products = { products: [{ id: "pie", name: "Pie", default_persona: null, repos: [], releases: [], tasks: 0, overrides: [] }] };
    const f = vi.fn(async (url: string, _init?: RequestInit) =>
      url.endsWith("/api/products") ? new Response(JSON.stringify(products)) : new Response(JSON.stringify({ id: 1, version: "1.0" })),
    );
    const client = new FleetClient("", f as unknown as typeof fetch);
    render(<FormsPage client={client} schema={schema} today="2026-10-29" />);
    await userEvent.type(await screen.findByLabelText("Version"), "1.0");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText("Created 1.0")).toBeInTheDocument();
    const [url, init] = f.mock.calls.find(([u]) => u.endsWith("/releases"))!;
    expect(url).toBe("/api/products/pie/releases");
    expect(JSON.parse(init?.body as string)).toEqual({ version: "1.0", start: "2026-10-29", target: "" });
  });

  it("says so when the products cannot be loaded", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ error: "store is busy" }), { status: 500 }));
    const client = new FleetClient("", f as unknown as typeof fetch);
    render(<FormsPage client={client} schema={schema} today="2026-10-29" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("store is busy");
  });

  it("edits the instructions every agent gets", async () => {
    const products = { products: [] };
    const config = { instructions: "Be brief.", settings: { allow: [], deny: [], env: {} } };
    const f = vi.fn(async (url: string, _init?: RequestInit) =>
      url.endsWith("/api/products") ? new Response(JSON.stringify(products)) : new Response(JSON.stringify(config)),
    );
    const client = new FleetClient("", f as unknown as typeof fetch);
    render(<FormsPage client={client} schema={schema} today="2026-10-29" />);
    const box = await screen.findByLabelText("Instructions for every agent");
    expect(box).toHaveValue("Be brief.");
    await userEvent.type(box, " Always.");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    const [, init] = f.mock.calls.find(([, i]) => i?.method === "PUT")!;
    expect(JSON.parse(init?.body as string)).toEqual({ instructions: "Be brief. Always." });
  });
});
