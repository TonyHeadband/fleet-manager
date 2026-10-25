import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Board } from "./Board";
import { Card } from "./Card";
import { PreviewPane } from "./PreviewPane";
import { RepoFilter } from "./RepoFilter";
import { SpawnForm } from "./SpawnForm";
import type { UiSchema } from "../api/generated/UiSchema";
import { FakeEventSource, agent, agentIn, schema, workspace } from "../test/fixtures";

const NOW = 1000 + 5 * 60;
beforeEach(() => FakeEventSource.reset());

describe("Board", () => {
  it("Board_renders_columns_from_schema_only", () => {
    const custom: UiSchema = {
      ...schema,
      columns: [
        { id: "left", title: "Alpha lane", statuses: ["working"], empty_hint: "nothing left", tone: "active" },
        { id: "right", title: "Beta lane", statuses: ["review"], empty_hint: "nothing right", tone: "done" },
      ],
    };
    render(<Board schema={custom} agents={[agent({ id: "w", status: "working" })]} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual(["Alpha lane", "Beta lane"]);
    expect(within(screen.getByRole("region", { name: "Alpha lane" })).getByText("fix the tests")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Beta lane" })).getByText("nothing right")).toBeInTheDocument();
    expect(screen.queryByText("Needs you")).not.toBeInTheDocument();
  });

  it("puts each agent in the column the schema maps its status to", () => {
    const agents = [agent({ id: "q", status: "queued", task: "t-q" }), agent({ id: "d", status: "stopped", task: "t-d" }), agent({ id: "n", status: "needs_you", task: "t-n" })];
    render(<Board schema={schema} agents={agents} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(within(screen.getByRole("region", { name: "Queued" })).getByText("t-q")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Review" })).getByText("t-d")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Needs you" })).getByText("t-n")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Talking" })).getByText("Agent conversations appear here")).toBeInTheDocument();
  });

  it("lists the longest-waiting card first", () => {
    const agents = [agent({ id: "new", task: "newer", status_since: 900 }), agent({ id: "old", task: "older", status_since: 100 })];
    render(<Board schema={schema} agents={agents} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const titles = within(screen.getByRole("region", { name: "Working" })).getAllByRole("article").map((c) => c.textContent);
    expect(titles[0]).toContain("older");
  });
});

describe("Card", () => {
  const needs = agent({ status: "needs_you", status_since: 1000, blocked_reason: "permission", activity: "wants to run Bash", paused: false });

  it("Card_shows_project_task_activity_and_time_waiting_for_needs_you", () => {
    render(<Card schema={schema} agent={needs} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const card = screen.getByRole("article");
    for (const t of ["core", "fix the tests", "wants to run Bash", "permission", "5m"]) expect(within(card).getByText(t)).toBeInTheDocument();
    expect(card).toHaveAttribute("data-tone", "attention");
  });

  it("Card_shows_one_badge_per_workspace_repo", () => {
    const two = agent({ workspaces: [workspace("core"), workspace("portal")] });
    render(<Card schema={schema} agent={two} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    const badges = screen.getAllByTitle("Repos").map((b) => b.textContent);
    expect(badges).toEqual(["core", "portal"]);
  });

  it("shows a changes badge when the agent has changes, and only then", () => {
    const { rerender } = render(<Card schema={schema} agent={agent({ status: "review", has_changes: true })} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.getByTitle("Has changes")).toHaveTextContent("Has changes");
    rerender(<Card schema={schema} agent={agent({ status: "review", has_changes: false })} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.queryByTitle("Has changes")).not.toBeInTheDocument();
  });

  it("shows only the fields whose condition holds", () => {
    render(<Card schema={schema} agent={agent({ status: "working", paused: false })} now={NOW} onRun={vi.fn()} onOpen={vi.fn()} />);
    expect(screen.queryByTitle("Blocked")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Paused")).not.toBeInTheDocument();
  });

  it("Card_answer_posts_text_and_keeps_literal_characters", async () => {
    const onRun = vi.fn();
    render(<Card schema={schema} agent={needs} now={NOW} onRun={onRun} onOpen={vi.fn()} />);
    await userEvent.click(screen.getByLabelText("Answer"));
    await userEvent.paste('$(x) "q" {a}'); // paste, not type: userEvent.type reads `{` as key syntax
    await userEvent.click(screen.getByRole("button", { name: "Answer" }));
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: "answer" }), needs, '$(x) "q" {a}');
  });

  it("does not send an empty answer", async () => {
    const onRun = vi.fn();
    render(<Card schema={schema} agent={needs} now={NOW} onRun={onRun} onOpen={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Answer" })).toBeDisabled();
  });

  it("asks for a second click before a destructive action", async () => {
    const onRun = vi.fn();
    render(<Card schema={schema} agent={agent()} now={NOW} onRun={onRun} onOpen={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Kill" }));
    expect(onRun).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Confirm Kill" }));
    expect(onRun).toHaveBeenCalledWith(expect.objectContaining({ id: "kill" }), expect.anything(), undefined);
  });

  it("opens the preview when the card body is clicked", async () => {
    const onOpen = vi.fn();
    render(<Card schema={schema} agent={agent()} now={NOW} onRun={vi.fn()} onOpen={onOpen} />);
    await userEvent.click(screen.getByText("fix the tests"));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }));
  });
});

describe("RepoFilter", () => {
  it("RepoFilter_toggles_repos_in_click_order", async () => {
    const onChange = vi.fn();
    const { rerender } = render(<RepoFilter repos={["core", "portal", "infra"]} selected={[]} onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: "core" }));
    expect(onChange).toHaveBeenLastCalledWith(["core"]);
    rerender(<RepoFilter repos={["core", "portal", "infra"]} selected={["core"]} onChange={onChange} />);
    expect(screen.getByRole("button", { name: "core" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "portal" }));
    expect(onChange).toHaveBeenLastCalledWith(["core", "portal"]);
    await userEvent.click(screen.getByRole("button", { name: "core" }));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });
});

describe("SpawnForm", () => {
  it("SpawnForm_posts_repo: spawns in the chosen repo with the typed prompt, keeping it literal", async () => {
    const onSpawn = vi.fn().mockResolvedValue(undefined);
    render(<SpawnForm repos={["core", "portal"]} onSpawn={onSpawn} />);
    expect(screen.getByRole("button", { name: "Start agent" })).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("Repo"), "portal");
    await userEvent.click(screen.getByLabelText("Prompt"));
    await userEvent.paste("fix $(x) {a}");
    await userEvent.click(screen.getByRole("button", { name: "Start agent" }));
    expect(onSpawn).toHaveBeenCalledWith("portal", "fix $(x) {a}");
  });

  it("shows the server's error and keeps the prompt", async () => {
    const onSpawn = vi.fn().mockRejectedValue(new Error('unknown project "x"'));
    render(<SpawnForm repos={["core"]} onSpawn={onSpawn} />);
    await userEvent.type(screen.getByLabelText("Prompt"), "do it");
    await userEvent.click(screen.getByRole("button", { name: "Start agent" }));
    expect(await screen.findByRole("alert")).toHaveTextContent('unknown project "x"');
    expect(screen.getByLabelText("Prompt")).toHaveValue("do it");
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
