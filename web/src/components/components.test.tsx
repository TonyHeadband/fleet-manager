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
import { SpawnForm } from "./SpawnForm";
import { FormsPage } from "./FormsPage";
import { FleetClient } from "../api/client";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, agent, agentIn, persona, repoInfo, schema, workspace } from "../test/fixtures";
import { title } from "../api/schema";
import { personaTags } from "../board/card";
import { spawnRequest } from "../shell/choices";

const NOW = 1000 + 5 * 60;
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

  it("a_stopped_card_offers_details_and_a_running_one_does_not", () => {
    draw(agent({ status: "stopped", stop_cause: "finished" }));
    const details = screen.getByRole("button", { name: "Details" });
    expect(details).toBeDisabled();
    expect(details).toHaveAttribute("title", "not on the web yet");
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
    expect(personaTags([agent({ persona: "ana" }), agent({ id: "a2", persona: "ana" })], people)).toEqual({});
    expect(personaTags([agent({ persona: "ana" }), agent({ id: "a2", persona: "gone" }), agent({ id: "a3", persona: "" })], people)).toEqual({
      ana: "ana",
      gone: "gone",
      "": "no persona",
    });
    const { card } = draw(agent(), { tag: "Ana" });
    expect(within(card).getByText("Ana")).toHaveAttribute("title", "Ana");
  });
});

describe("Task cards", () => {
  // Ported from crates/fleet-gui/tests/board.rs; the names are Lite's.
  const o = (over: Partial<Agent> = {}) => agent({ id: "o", kind: "orchestrator", goal: "Fix login", status: "working", ...over });
  const f = (id: string, over: Partial<Agent> = {}) => agent({ id, kind: "specialist", task: "o", role: "rust", goal: `step ${id}`, status: "working", ...over });
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

describe("RepoFilter", () => {
  it("RepoFilter_toggles_repos_in_click_order", async () => {
    const onChange = vi.fn();
    const { rerender } = render(<RepoFilter repos={[["core", 2], ["portal", 1], ["infra", 0]]} selected={[]} onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: /^core/ }));
    expect(onChange).toHaveBeenLastCalledWith(["core"]);
    expect(screen.getByRole("button", { name: "core 2" })).toBeInTheDocument();
    rerender(<RepoFilter repos={[["core", 2], ["portal", 1], ["infra", 0]]} selected={["core"]} onChange={onChange} />);
    expect(screen.getByRole("button", { name: /^core/ })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: /^portal/ }));
    expect(onChange).toHaveBeenLastCalledWith(["core", "portal"]);
    await userEvent.click(screen.getByRole("button", { name: /^core/ }));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });
});

describe("SpawnForm", () => {
  const repos = [repoInfo("core"), repoInfo("portal"), repoInfo("lab", { url: null, path: "/src/lab" })];
  const open = (onSpawn = vi.fn().mockResolvedValue(undefined), onClose = vi.fn()) => {
    render(<SpawnForm product="p1" persona="me" repos={repos} onSpawn={onSpawn} onClose={onClose} />);
    return { onSpawn, onClose };
  };

  it("new_task_sends_a_request_the_daemon_accepts", async () => {
    const { onSpawn } = open();
    expect(screen.getByRole("dialog", { name: "New task" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start task" })).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: "core" }));
    await userEvent.click(screen.getByLabelText("Goal"));
    await userEvent.paste("fix $(x) {a}");
    await userEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(onSpawn).toHaveBeenCalledWith(spawnRequest("p1", "me", ["core"], "fix $(x) {a}"));
  });

  it("a_local_only_repo_cannot_be_ticked_and_says_why", () => {
    open();
    expect(screen.getByRole("checkbox", { name: "lab" })).toBeDisabled();
    expect(screen.getByText("lab").closest("label")).toHaveAttribute("title", "start it from Lite for now");
  });

  it("ctrl_enter_starts_the_task", async () => {
    const { onSpawn } = open();
    await userEvent.type(screen.getByLabelText("Goal"), "go{Control>}{Enter}{/Control}");
    expect(onSpawn).toHaveBeenCalledWith(spawnRequest("p1", "me", [], "go"));
  });

  it("a_refusal_stays_in_the_dialog", async () => {
    const { onClose } = open(vi.fn().mockRejectedValue(new Error('unknown repo "x"')));
    await userEvent.type(screen.getByLabelText("Goal"), "do it");
    await userEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(await within(screen.getByRole("dialog")).findByRole("alert")).toHaveTextContent('unknown repo "x"');
    expect(screen.getByLabelText("Goal")).toHaveValue("do it");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("cancel_and_escape_close_it", async () => {
    const { onClose } = open();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
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
