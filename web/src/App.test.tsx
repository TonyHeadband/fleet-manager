import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type { FleetClient } from "./api/client";
import { spawnRequest } from "./shell/choices";
import { FakeEventSource, agent, agentIn, factory, fakeClient, persona, product, repoInfo, schema } from "./test/fixtures";

beforeEach(() => {
  FakeEventSource.reset();
  window.location.hash = "";
});

describe("App", () => {
  it("New_task_says_why_while_the_daemon_is_down", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    localStorage.setItem("fleet.product", "p1");
    const personas = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue({ personas: [persona("me")] });
    const products = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue({ products: [product("p1")] });
    const uiSchema = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue(schema);
    const repos = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue([repoInfo("core")]);
    render(<App client={fakeClient({ personas, products, uiSchema, repos })} eventSource={factory} nowSeconds={() => 1300} />);
    const newTask = await screen.findByRole("button", { name: "New task" });
    await waitFor(() => expect(newTask).toHaveAttribute("title", "Could not load the personas; retrying"));
    expect(newTask).toBeDisabled();
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(newTask).toBeEnabled();
    // The board and the product's repos come back too, not only the drawer.
    expect(screen.getByRole("region", { name: "Working" })).toBeInTheDocument();
    await userEvent.click(newTask);
    expect(await screen.findByRole("button", { name: "core" })).toBeInTheDocument();
    vi.useRealTimers();
    localStorage.clear();
  });

  it("App_moves_a_card_when_a_status_event_arrives", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const es = FakeEventSource.for("/api/events");
    act(() => es.emit("snapshot", { seq: 1, agents: [agent({ id: "a1", status: "working" })] }, "1"));
    expect(within(screen.getByRole("region", { name: "Working" })).getByText("fix the tests")).toBeInTheDocument();
    act(() =>
      es.emit("event", { seq: 2, kind: { type: "status_changed", id: "a1", status: "needs_you", activity: "wants to run Bash", since: 1200, has_changes: false, paused: false, blocked_reason: "permission" } }, "2"),
    );
    expect(within(screen.getByRole("region", { name: "Needs you" })).getByText("fix the tests")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Working" })).queryByText("fix the tests")).not.toBeInTheDocument();
  });

  it("RepoFilter_shows_agents_touching_any_selected_repo", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const es = FakeEventSource.for("/api/events");
    act(() => es.emit("snapshot", { seq: 1, agents: [agentIn("core", { id: "a", goal: "core task" }), agentIn("portal", { id: "b", goal: "portal task" }), agent({ id: "c", goal: "both task", workspaces: [{ repo: "core", branch: "b", worktree: "/w", base: "x", has_changes: false, owned: false }, { repo: "portal", branch: "b", worktree: "/w2", base: "x", has_changes: false, owned: false }] })] }, "1"));
    await userEvent.click(screen.getByRole("button", { name: "Repo filter" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "portal" }));
    expect(screen.queryByText("core task")).not.toBeInTheDocument();
    expect(screen.getByText("portal task")).toBeInTheDocument();
    expect(screen.getByText("both task")).toBeInTheDocument();
  });

  describe("header filters", () => {
    const pick = async (filter: string, row: string) => {
      await userEvent.click(screen.getByRole("button", { name: filter }));
      await userEvent.click(screen.getByRole(filter === "Release filter" ? "button" : "checkbox", { name: row }));
      await userEvent.keyboard("{Escape}");
    };
    const snapshot = (agents: ReturnType<typeof agent>[]) => act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));

    it("the_header_holds_lites_controls_in_order", async () => {
      render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      const header = document.querySelector<HTMLElement>(".view-header")!;
      const order = [...header.querySelectorAll("h1, [aria-haspopup], .fleet-switch > .mono-label")].map((e) => e.getAttribute("aria-label") ?? e.textContent?.split(/\s/)[0]);
      expect(order).toEqual(["Kanban", "Repo filter", "Persona filter", "Release filter", "Fleet"]);
    });

    it("persona_filter_hides_other_cards", async () => {
      const personas = vi.fn().mockResolvedValue({ personas: [{ ...persona("me"), name: "Me" }, { ...persona("work"), name: "Work" }] });
      render(<App client={fakeClient({ personas })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      snapshot([agent({ id: "a", goal: "mine", persona: "me" }), agent({ id: "b", goal: "for work", persona: "work" })]);
      await pick("Persona filter", "Work");
      expect(screen.queryByText("mine")).toBeNull();
      expect(screen.getByText("for work")).toBeInTheDocument();
      expect(screen.getByRole("combobox", { name: "Active persona" }), "the active persona is another thing").toHaveValue("me");
      await pick("Persona filter", "All personas");
      expect(screen.getByText("mine")).toBeInTheDocument();
    });

    it("a_renamed_persona_stays_selected_and_a_deleted_one_is_dropped", async () => {
      // "work" is no longer a ready persona: only its agent keeps it in the filter.
      const personas = vi.fn().mockResolvedValue({ personas: [{ ...persona("me"), name: "Me" }, { ...persona("work", false), name: "Work" }] });
      render(<App client={fakeClient({ personas })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      snapshot([agent({ id: "a", goal: "mine", persona: "me" }), agent({ id: "b", goal: "for work", persona: "work" })]);
      await pick("Persona filter", "Work");
      expect(screen.queryByText("mine")).toBeNull();
      // The work persona and its agent are gone: the pick must not keep hiding the rest.
      act(() => FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "agent_removed", id: "b" } }, "2"));
      expect(screen.getByText("mine")).toBeInTheDocument();
    });

    it("release_filter_backlog_hides_assigned_tasks", async () => {
      localStorage.setItem("fleet.product", "p1");
      const products = vi.fn().mockResolvedValue({ products: [product("p1", "Pie", { releases: [{ id: 1, product: "p1", version: "1.0", start: "", target: "", released_at: null }] })] });
      const tasks = vi.fn().mockResolvedValue({ tasks: [{ id: "a", release: 1 }, { id: "b", release: null }], orphans: [], clones_bytes: null });
      render(<App client={fakeClient({ products, tasks })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      snapshot([agent({ id: "a", goal: "in 1.0", product: "p1" }), agent({ id: "b", goal: "in backlog", product: "p1" })]);
      await waitFor(() => expect(tasks).toHaveBeenCalled());
      await pick("Release filter", "Backlog");
      expect(screen.queryByText("in 1.0")).toBeNull();
      expect(screen.getByText("in backlog")).toBeInTheDocument();
      await pick("Release filter", "1.0");
      expect(screen.getByText("in 1.0")).toBeInTheDocument();
      expect(screen.queryByText("in backlog")).toBeNull();
      localStorage.clear();
    });

    it("the_repo_filter_survives_a_product_change", async () => {
      const products = vi.fn().mockResolvedValue({ products: [product("p1", "Pie"), product("p2", "Tart")] });
      render(<App client={fakeClient({ products })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      snapshot([agentIn("core", { id: "a", goal: "core task" }), agentIn("portal", { id: "b", goal: "portal task" })]);
      await pick("Repo filter", "portal");
      await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Active product" }), "p2");
      expect(screen.getByRole("button", { name: "Repo filter" })).toHaveTextContent("portal");
      localStorage.clear();
    });
  });

  it("runs a card action through the client and shows a failure", async () => {
    const run = vi.fn().mockRejectedValue(new Error("agent is not running"));
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(run).toHaveBeenCalledWith(expect.objectContaining({ id: "pause" }), expect.objectContaining({ id: "a" }), undefined);
    // A failure goes to the footer's message line, as in Lite.
    expect(await within(screen.getByRole("contentinfo")).findByText("agent is not running")).toBeInTheDocument();
  });

  it("opens a read-only preview for the clicked card", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    expect(FakeEventSource.for("/api/agents/a/terminal")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Diff" })).toBeInTheDocument();
  });

  it("the_panel_gets_the_reply_box_for_a_waiting_agent", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", status: "needs_you", blocked_reason: "question" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    const box = within(screen.getByRole("complementary", { name: "Detail" })).getByPlaceholderText("Answer…");
    expect(box).toHaveFocus();
  });

  it("a_working_agent_gets_no_reply_box", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    expect(within(screen.getByRole("complementary", { name: "Detail" })).queryByRole("textbox")).toBeNull();
  });

  it("ctrl_enter_sends_the_reply", async () => {
    const run = vi.fn().mockResolvedValue(undefined);
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", status: "needs_you", blocked_reason: "question" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    await userEvent.type(screen.getByPlaceholderText("Answer…"), "yes{Control>}{Enter}{/Control}");
    expect(run).toHaveBeenCalledWith(expect.objectContaining({ id: "answer" }), expect.objectContaining({ id: "a" }), "yes");
    await waitFor(() => expect(screen.getByPlaceholderText("Answer…")).toHaveValue(""));
  });

  describe("live data", () => {
    async function started(over = {}) {
      const client = fakeClient(over);
      render(<App client={client} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      return { client, es: FakeEventSource.for("/api/events") };
    }

    it("a_reconnect_reloads_the_schema_and_repos", async () => {
      const { client, es } = await started();
      act(() => es.emit("open", null));
      expect(client.uiSchema).toHaveBeenCalledTimes(1);
      act(() => {
        es.emit("error", null);
        es.emit("open", null);
      });
      await waitFor(() => expect(client.uiSchema).toHaveBeenCalledTimes(2));
      expect(client.repos).toHaveBeenCalledTimes(2);
    });

    it("a_snapshot_refetches_the_tasks", async () => {
      const { client, es } = await started();
      await waitFor(() => expect(client.tasks).toHaveBeenCalledTimes(1));
      act(() => es.emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
      await waitFor(() => expect(client.tasks).toHaveBeenCalledTimes(2));
      act(() => es.emit("event", { seq: 2, kind: { type: "agent_created", agent: agent({ id: "b" }) } }, "2"));
      await waitFor(() => expect(client.tasks).toHaveBeenCalledTimes(3));
      act(() => es.emit("event", { seq: 3, kind: { type: "status_changed", id: "a", status: "needs_you", activity: "", since: 1200, has_changes: false, paused: false, blocked_reason: "question" } }, "3"));
      act(() => es.emit("event", { seq: 4, kind: { type: "status_changed", id: "a", status: "stopped", activity: "", since: 1250, has_changes: false, paused: false, blocked_reason: null } }, "4"));
      await waitFor(() => expect(client.tasks, "only the stop counts").toHaveBeenCalledTimes(4));
    });

    it("an_unknown_persona_refetches_the_list_once", async () => {
      const personas = vi.fn().mockResolvedValue({ personas: [persona("me")] });
      const { es } = await started({ personas });
      await waitFor(() => expect(personas).toHaveBeenCalledTimes(1));
      act(() => es.emit("snapshot", { seq: 1, agents: [agent({ id: "a", persona: "new" })] }, "1"));
      await waitFor(() => expect(personas).toHaveBeenCalledTimes(2));
      act(() => es.emit("snapshot", { seq: 2, agents: [agent({ id: "a", persona: "new" }), agent({ id: "b", persona: "new" })] }, "2"));
      await act(async () => {});
      expect(personas, "once per id").toHaveBeenCalledTimes(2);
    });
  });

  describe("a repo request", () => {
    const waiting = (over = {}) => agent({ id: "a", product: "p1", status: "needs_you", blocked_reason: "repo_access", repo_request: { repo: "portal", why: "login" }, ...over });
    async function openOn(client: FleetClient, a = waiting()) {
      render(<App client={client} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [a] }, "1"));
      await userEvent.click(screen.getByText("fix the tests"));
      await screen.findByRole("button", { name: "portal" });
      return within(screen.getByRole("complementary", { name: "Detail" }));
    }

    it("a_pending_request_takes_the_panel_instead_of_the_reply_box", async () => {
      const panel = await openOn(fakeClient());
      expect(panel.getByText("Wants portal")).toBeInTheDocument();
      expect(panel.queryByPlaceholderText("Answer…")).toBeNull();
      expect(panel.getByLabelText("Reason (optional)")).toHaveFocus();
    });

    it("grant_and_refuse_go_through_the_client_and_failures_show", async () => {
      const grantRepo = vi.fn().mockRejectedValue(new Error("repo is gone"));
      const refuseRepo = vi.fn().mockResolvedValue(undefined);
      const panel = await openOn(fakeClient({ grantRepo, refuseRepo }));
      await userEvent.click(panel.getByRole("button", { name: "Grant" }));
      expect(grantRepo).toHaveBeenCalledWith("a", "portal");
      expect(await within(screen.getByRole("contentinfo")).findByText("repo is gone")).toBeInTheDocument();
      await userEvent.type(panel.getByLabelText("Reason (optional)"), " no ");
      await userEvent.click(panel.getByRole("button", { name: "Refuse" }));
      expect(refuseRepo).toHaveBeenCalledWith("a", "no");
    });

    it("a_picked_grant_target_survives_a_failed_grant", async () => {
      const grantRepo = vi.fn().mockRejectedValue(new Error("nope"));
      const panel = await openOn(fakeClient({ grantRepo, repos: vi.fn().mockResolvedValue([repoInfo("core"), repoInfo("portal")]) }));
      await userEvent.click(panel.getByRole("button", { name: "core" }));
      await userEvent.type(panel.getByLabelText("Reason (optional)"), "kept");
      await userEvent.click(panel.getByRole("button", { name: "Grant" }));
      await within(screen.getByRole("contentinfo")).findByText("nope");
      expect(grantRepo).toHaveBeenCalledWith("a", "core");
      expect(panel.getByRole("button", { name: "core" })).toHaveAttribute("aria-pressed", "true");
      expect(panel.getByLabelText("Reason (optional)")).toHaveValue("kept");
    });

    it("a_decided_requests_draft_goes_with_it", async () => {
      const panel = await openOn(fakeClient({ repos: vi.fn().mockResolvedValue([repoInfo("core"), repoInfo("portal")]) }));
      await userEvent.click(panel.getByRole("button", { name: "core" }));
      await userEvent.type(panel.getByLabelText("Reason (optional)"), "old");
      // Decided (here or elsewhere): the request leaves the snapshot. A later one for the same repo starts afresh, as Lite's.
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 2, agents: [waiting({ status: "working", blocked_reason: null, repo_request: null })] }, "2"));
      await waitFor(() => expect(panel.queryByText("Wants portal")).toBeNull());
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 3, agents: [waiting()] }, "3"));
      expect(await panel.findByRole("button", { name: "portal" })).toHaveAttribute("aria-pressed", "true");
      expect(panel.getByRole("button", { name: "core" })).toHaveAttribute("aria-pressed", "false");
      expect(panel.getByLabelText("Reason (optional)")).toHaveValue("");
    });

    it("a_second_grant_while_one_is_in_flight_is_not_sent", async () => {
      const grantRepo = vi.fn(() => new Promise<void>(() => {}));
      const panel = await openOn(fakeClient({ grantRepo }));
      await userEvent.click(panel.getByRole("button", { name: "Grant" }));
      expect(panel.getByRole("button", { name: "Grant" })).toBeDisabled();
      expect(panel.getByRole("button", { name: "Grant" })).toHaveAttribute("title", "A decision is being sent");
      expect(panel.getByRole("button", { name: "Refuse" })).toBeDisabled();
      expect(grantRepo).toHaveBeenCalledTimes(1);
    });

    it("repo_granted_is_announced_for_a_grant_and_not_for_a_refusal", async () => {
      const repos = vi.fn().mockResolvedValue([repoInfo("core"), repoInfo("portal")]);
      const panel = await openOn(fakeClient({ grantRepo: vi.fn().mockResolvedValue({}), refuseRepo: vi.fn().mockResolvedValue(undefined), repos }));
      const footer = within(screen.getByRole("contentinfo"));
      await userEvent.click(panel.getByRole("button", { name: "Refuse" }));
      await waitFor(() => expect(repos).toHaveBeenCalledTimes(2));
      expect(footer.queryByText("Repo granted")).toBeNull();
      await userEvent.click(panel.getByRole("button", { name: "Grant" }));
      expect(await footer.findByText("Repo granted")).toBeInTheDocument();
      // A decision changes what the product's repos hold: they are asked for again, as Lite's refresh_repos.
      await waitFor(() => expect(repos).toHaveBeenCalledTimes(3));
    });

    it("a_stopped_agents_panel_has_no_reply_box", async () => {
      render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [waiting({ status: "stopped" })] }, "1"));
      await userEvent.click(screen.getByText("fix the tests"));
      const panel = within(screen.getByRole("complementary", { name: "Detail" }));
      expect(panel.queryByText("Wants portal")).toBeNull();
      expect(panel.queryByRole("textbox")).toBeNull();
    });
  });

  it("a_reply_box_that_appears_later_does_not_take_the_focus", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    act(() => FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "status_changed", id: "a", status: "needs_you", activity: "asks", since: 1200, has_changes: false, paused: false, blocked_reason: "question" } }, "2"));
    const box = await screen.findByPlaceholderText("Answer…");
    expect(box).not.toHaveFocus();
  });

  it("a_failed_reply_keeps_the_draft_and_tells_the_footer", async () => {
    const run = vi.fn().mockRejectedValue(new Error("agent is not running"));
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", status: "needs_you", blocked_reason: "question" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    await userEvent.type(screen.getByPlaceholderText("Answer…"), "yes{Control>}{Enter}{/Control}");
    expect(await within(screen.getByRole("contentinfo")).findByText("agent is not running")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Answer…")).toHaveValue("yes");
  });

  it("a_reply_in_flight_is_sent_once_and_text_typed_meanwhile_survives", async () => {
    let finish!: () => void;
    const run = vi.fn(() => new Promise<void>((r) => (finish = r)));
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", status: "needs_you", blocked_reason: "question" })] }, "1"));
    await userEvent.click(screen.getByText("fix the tests"));
    const box = screen.getByPlaceholderText("Answer…");
    await userEvent.type(box, "yes{Control>}{Enter}{/Control}");
    await userEvent.type(box, "{Control>}{Enter}{/Control}");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(run).toHaveBeenCalledTimes(1);
    await userEvent.type(box, " and more");
    await act(async () => finish());
    expect(box).toHaveValue("yes and more");
  });

  it("a_sent_reply_clears_only_that_agents_draft", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const waiting = { status: "needs_you", blocked_reason: "question" } as const;
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a1", goal: "first", ...waiting }), agent({ id: "a2", goal: "second", ...waiting })] }, "1"));
    const board = screen.getByRole("region", { name: "Needs you" });
    await userEvent.click(within(board).getByText("first"));
    await userEvent.type(screen.getByPlaceholderText("Answer…"), "kept");
    await userEvent.click(within(board).getByText("second"));
    await userEvent.type(screen.getByPlaceholderText("Answer…"), "sent{Control>}{Enter}{/Control}");
    await waitFor(() => expect(screen.getByPlaceholderText("Answer…")).toHaveValue(""));
    await userEvent.click(within(board).getByText("first"));
    expect(screen.getByPlaceholderText("Answer…")).toHaveValue("kept");
  });

  it("each_agent_keeps_its_own_draft", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    const waiting = { status: "needs_you", blocked_reason: "question" } as const;
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a1", goal: "first", ...waiting }), agent({ id: "a2", goal: "second", ...waiting })] }, "1"));
    const board = screen.getByRole("region", { name: "Needs you" });
    await userEvent.click(within(board).getByText("first"));
    await userEvent.type(screen.getByPlaceholderText("Answer…"), "for one");
    await userEvent.click(within(board).getByText("second"));
    expect(screen.getByPlaceholderText("Answer…")).toHaveValue("");
    await userEvent.click(within(board).getByText("first"));
    expect(screen.getByPlaceholderText("Answer…")).toHaveValue("for one");
  });

  describe("a task's own controls", () => {
    const o = (over = {}) => agent({ id: "o", kind: "orchestrator", goal: "Fix login", status: "working", ...over });
    const f = (id: string, over = {}) => agent({ id, kind: "specialist", task: "o", role: "rust", goal: `step ${id}`, status: "working", ...over });
    const start = async (agents: ReturnType<typeof agent>[], over: Parameters<typeof fakeClient>[0] = {}) => {
      // A board with tasks has a ready persona; without one the persona setup would hold the dialog slot.
      const client = fakeClient({
        personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }),
        stopTask: vi.fn().mockResolvedValue({ stopped: 3 }),
        restartOrchestrator: vi.fn().mockResolvedValue(o()),
        ...over,
      });
      render(<App client={client} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
      return client;
    };

    it("task_stop_confirms_with_count", async () => {
      const client = await start([o(), f("f1"), f("f2")]);
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      const dialog = screen.getByRole("dialog", { name: "Stop the orchestrator and 2 fleet agents?" });
      expect(dialog).toHaveTextContent("Their sessions end. Folders and branches stay, so the task can be archived later.");
      await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(client.stopTask).not.toHaveBeenCalled();
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      await userEvent.click(screen.getByRole("button", { name: "Stop all" }));
      expect(client.stopTask).toHaveBeenCalledWith("o");
      expect(await within(screen.getByRole("contentinfo")).findByText("Fix login: 3 agents stopped")).toBeInTheDocument();
    });

    it("the_dialog_names_what_is_left_to_stop", async () => {
      await start([o({ status: "stopped" }), f("f1")]);
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      expect(screen.getByRole("dialog", { name: "Stop 1 fleet agent?" })).toBeInTheDocument();
    });

    it("stop_all_twice_stops_once_and_escape_cancels", async () => {
      const client = await start([o(), f("f1")]);
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      await userEvent.keyboard("{Escape}");
      expect(screen.queryByRole("dialog")).toBeNull();
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      await userEvent.dblClick(screen.getByRole("button", { name: "Stop all" }));
      expect(client.stopTask).toHaveBeenCalledTimes(1);
    });

    it("restart_offered_once_the_lead_stopped", async () => {
      const client = await start([o({ status: "stopped", stop_cause: "stopped" }), f("f1", { status: "stopped", stop_cause: "stopped" })]);
      await userEvent.click(screen.getByRole("button", { name: "Restart orchestrator" }));
      expect(client.restartOrchestrator).toHaveBeenCalledWith("o");
      expect(await within(screen.getByRole("contentinfo")).findByText("Orchestrator restarted")).toBeInTheDocument();
    });

    it("a_restarted_orchestrator_that_crashes_asks_for_a_restart", async () => {
      // Stopped by Stop task, restarted as o2, then o2 crashed: o2 still leads, and the task needs you.
      await start([
        o({ status: "stopped", stop_cause: "stopped" }),
        f("f1", { status: "stopped", stop_cause: "stopped" }),
        o({ id: "o2", task: "o", goal: "Fix login again", status: "working" }),
      ]);
      act(() =>
        FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "status_changed", id: "o2", status: "stopped", activity: "", since: 1250, has_changes: false, paused: false, blocked_reason: null, stop_cause: "crashed" } }, "2"),
      );
      const card = within(screen.getByRole("region", { name: "Needs you" })).getByRole("group", { name: "Fix login again, 2 fleet agents" });
      expect(within(card).getByText("Orchestrator stopped · Restart?")).toBeInTheDocument();
      expect(within(screen.getByRole("region", { name: "Stopped" })).queryByRole("group")).toBeNull();
    });

    it("a_failed_task_stop_tells_the_footer", async () => {
      await start([o(), f("f1")], { stopTask: vi.fn().mockRejectedValue(new Error("store is busy")) });
      await userEvent.click(screen.getByRole("button", { name: "Stop" }));
      await userEvent.click(screen.getByRole("button", { name: "Stop all" }));
      expect(await within(screen.getByRole("contentinfo")).findByText("Could not stop Fix login: store is busy")).toBeInTheDocument();
    });

    it("a_failed_restart_tells_the_footer", async () => {
      await start([o({ status: "stopped", stop_cause: "crashed" })], { restartOrchestrator: vi.fn().mockRejectedValue(new Error("login expired")) });
      await userEvent.click(screen.getByRole("button", { name: "Restart orchestrator" }));
      expect(await within(screen.getByRole("contentinfo")).findByText("Could not restart the orchestrator of Fix login: login expired")).toBeInTheDocument();
    });
  });

  it("the_fleet_switch_marks_the_current_chip", async () => {
    localStorage.removeItem("fleet.display");
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    const header = document.querySelector<HTMLElement>(".view-header")!;
    expect(within(header).getByText("Fleet")).toHaveClass("mono-label");
    expect(within(header).getByRole("button", { name: "Compact" })).toHaveAttribute("aria-pressed", "true");
    expect(within(header).getByRole("button", { name: "Show" })).toHaveAttribute("aria-pressed", "false");
  });

  it("display_choice_is_saved", async () => {
    localStorage.removeItem("fleet.display");
    const { unmount } = render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await userEvent.click(screen.getByRole("button", { name: "Show" }));
    expect(localStorage.getItem("fleet.display")).toBe("show");
    expect(screen.getByRole("button", { name: "Show" })).toHaveAttribute("aria-pressed", "true");
    unmount();
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    expect(screen.getByRole("button", { name: "Show" })).toHaveAttribute("aria-pressed", "true");
    localStorage.removeItem("fleet.display");
  });

  describe("the detail panel", () => {
    const openOn = async (agents: ReturnType<typeof agent>[], click: string) => {
      render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
      await userEvent.click(screen.getAllByText(click)[0]!);
      return screen.getByRole("complementary", { name: "Detail" });
    };

    it("opening_a_card_fills_the_panel_path", async () => {
      const panel = await openOn([agent({ id: "a" })], "fix the tests");
      expect(within(panel).getByText("core / fleet/fix-the-tests")).toBeInTheDocument();
    });

    it("the_panel_title_is_the_agents_title_and_the_goal_follows", async () => {
      const goal = "Fix the flaky tests in the payments service. Then look at the retry logic too.";
      const panel = await openOn([agent({ id: "a", goal, title: "Flaky payments tests" })], "Flaky payments tests");
      expect(within(panel).getByRole("heading", { name: "Flaky payments tests" })).toBeInTheDocument();
      expect(within(panel).getByText(goal)).toHaveClass("detail-goal");
    });

    it("a_goal_that_is_its_own_title_shows_once", async () => {
      const panel = await openOn([agent({ id: "a", goal: "fix the tests" })], "fix the tests");
      expect(within(panel).getAllByText("fix the tests")).toHaveLength(1);
    });

    it("the_details_sit_between_the_title_and_the_tabs", async () => {
      const panel = await openOn([agent({ id: "a" })], "fix the tests");
      const order = [...panel.querySelectorAll(".detail-title, .details, .repos-row, [role='tablist']")].map((e) => e.className || e.getAttribute("role"));
      expect(order).toEqual(["detail-title", "details", "repos-row", "tabs"]);
    });

    it("a_status_event_updates_the_open_panel", async () => {
      const panel = await openOn([agent({ id: "a" })], "fix the tests");
      act(() =>
        FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "status_changed", id: "a", status: "needs_you", activity: "", since: 1200, has_changes: false, paused: false, blocked_reason: "permission" } }, "2"),
      );
      const details = panel.querySelector<HTMLElement>(".details")!;
      expect(within(details).getByText("Needs you")).toBeInTheDocument();
      expect(within(details).getByText("Permission needed")).toBeInTheDocument();
    });

    it("the_board_marks_the_pr_from_the_fleet_view", async () => {
      const fleetView = vi.fn().mockResolvedValue({ task: "o", breakdowns: [], gate_runs: [{ at: 0, base: "b", steps: [], unmatched: [], pr_url: "https://pr/1" }] });
      render(<App client={fakeClient({ fleetView })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      const fleet = [agent({ id: "o", kind: "orchestrator", goal: "Fix login" }), agent({ id: "f", kind: "specialist", task: "o", role: "rust", goal: "drop cookie" })];
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: fleet }, "1"));
      expect(within(screen.getByRole("button", { name: "rust drop cookie, working" })).queryByRole("img", { name: "PR open" })).toBeNull();
      await userEvent.click(screen.getByText("Fix login"));
      expect(fleetView).toHaveBeenCalledWith("o");
      expect(await within(screen.getByRole("button", { name: "rust drop cookie, working" })).findByRole("img", { name: "PR open" })).toBeInTheDocument();
      // Any change among the task's agents asks for the view again, as Lite's follow_fleet_event.
      act(() =>
        FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "status_changed", id: "f", status: "review", activity: "", since: 1250, has_changes: false, paused: false, blocked_reason: null } }, "2"),
      );
      expect(fleetView).toHaveBeenCalledTimes(2);
    });

    it("specialist_panel_links_to_task", async () => {
      const fleet = [agent({ id: "o", kind: "orchestrator", goal: "Fix login" }), agent({ id: "f", kind: "specialist", task: "o", role: "rust", goal: "drop cookie" })];
      const panel = await openOn(fleet, "drop cookie");
      await userEvent.click(within(panel).getByRole("button", { name: "↳ Fix login" }));
      expect(within(screen.getByRole("complementary", { name: "Detail" })).getByRole("heading", { name: "Fix login" })).toBeInTheDocument();
    });

    it("breakdown_tab_first_when_pending", async () => {
      const proposed = { task: "o", breakdowns: [{ version: 2, steps: [], approval: "proposed", comment: null, proposed_at: 0 }], gate_runs: [] };
      const decideBreakdown = vi.fn().mockResolvedValue(undefined);
      const fleetView = vi.fn().mockResolvedValue(proposed);
      render(<App client={fakeClient({ fleetView, decideBreakdown })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      const waiting = agent({ id: "o", kind: "orchestrator", goal: "Fix login", status: "needs_you", blocked_reason: "breakdown" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [waiting, agent({ id: "x", goal: "other task" })] }, "1"));
      await userEvent.click(screen.getByText("Fix login"));
      const panel = screen.getByRole("complementary", { name: "Detail" });
      expect(within(panel).getAllByRole("tab").map((t) => t.textContent)).toEqual(["Breakdown", "Terminal", "Diff"]);
      expect(within(panel).getByRole("tab", { name: "Breakdown" })).toHaveAttribute("aria-selected", "true");
      expect(await within(panel).findByText("v2 · proposed")).toBeInTheDocument();
      // A decision goes once, with the version on screen, and the controls wait for a fresh view.
      await userEvent.click(within(panel).getByRole("button", { name: "Approve" }));
      expect(decideBreakdown).toHaveBeenCalledWith("o", { approve: true, comment: null, version: 2 });
      expect(await within(screen.getByRole("contentinfo")).findByText("Breakdown approved")).toBeInTheDocument();
      expect(fleetView, "the view is asked for again").toHaveBeenCalledTimes(2);
      // The solo agent's panel has no Breakdown.
      await userEvent.click(screen.getByText("other task"));
      expect(within(screen.getByRole("complementary", { name: "Detail" })).queryByRole("tab", { name: "Breakdown" })).toBeNull();
    });

    it("a_second_decision_while_one_is_on_its_way_is_not_sent", async () => {
      const proposed = { task: "o", breakdowns: [{ version: 2, steps: [], approval: "proposed", comment: null, proposed_at: 0 }], gate_runs: [] };
      const decideBreakdown = vi.fn(() => new Promise<void>(() => {}));
      render(<App client={fakeClient({ fleetView: vi.fn().mockResolvedValue(proposed), decideBreakdown })} eventSource={factory} nowSeconds={() => 1300} />);
      await screen.findByRole("region", { name: "Working" });
      act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "o", kind: "orchestrator", goal: "Fix login" })] }, "1"));
      await userEvent.click(screen.getByText("Fix login"));
      await userEvent.click(screen.getByRole("tab", { name: "Breakdown" }));
      await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
      expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
      expect(decideBreakdown).toHaveBeenCalledTimes(1);
    });

    it("the_preview_closes_when_its_agent_is_removed", async () => {
      await openOn([agent({ id: "a" })], "fix the tests");
      act(() => FakeEventSource.for("/api/events").emit("event", { seq: 2, kind: { type: "agent_removed", id: "a" } }, "2"));
      expect(screen.queryByRole("complementary", { name: "Detail" })).toBeNull();
    });
  });

  it("App_switches_between_the_board_and_the_forms_page", async () => {
    const products = vi.fn().mockResolvedValue({ products: [{ id: "pie", name: "Pie", default_persona: null, repos: [], releases: [], tasks: 0, overrides: [] }] });
    const agentConfig = vi.fn().mockResolvedValue({ instructions: "", settings: { allow: [], deny: [], env: {} } });
    render(<App client={fakeClient({ products, agentConfig })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    // The debug page is reached by its address; the drawer lists only Lite's views.
    window.location.hash = "#forms";
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(await screen.findByRole("heading", { name: "New release" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Working" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("link", { name: "Kanban" }));
    act(() => window.dispatchEvent(new HashChangeEvent("hashchange")));
    expect(await screen.findByRole("region", { name: "Working" })).toBeInTheDocument();
  });

  it("App_has_no_spawn_bar", async () => {
    localStorage.setItem("fleet.product", "p1");
    const spawn = vi.fn().mockResolvedValue({ agent: agent(), warnings: [] });
    render(
      <App
        client={fakeClient({ spawn, personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }), products: vi.fn().mockResolvedValue({ products: [product("p1")] }) })}
        eventSource={factory}
        nowSeconds={() => 1300}
      />,
    );
    const newTask = await screen.findByRole("button", { name: "New task" });
    expect(screen.queryByRole("button", { name: "Start task" })).not.toBeInTheDocument();
    await waitFor(() => expect(newTask).toBeEnabled());
    await userEvent.click(newTask);
    await userEvent.click(await screen.findByRole("button", { name: "core" }));
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    await userEvent.click(screen.getByRole("button", { name: "Start task" }));
    expect(spawn).toHaveBeenCalledWith({ ...spawnRequest("p1", "me", ["core"], "go"), release: null, title: null, kind: "solo", role: null });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    localStorage.clear();
  });

  it("a_started_task_and_its_warnings_reach_the_footer", async () => {
    localStorage.setItem("fleet.product", "p1");
    const spawn = vi.fn().mockResolvedValue({ agent: agent({ id: "a9" }), warnings: ["could not fetch core; started from the last fetched commit"] });
    render(
      <App
        client={fakeClient({ spawn, personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }), products: vi.fn().mockResolvedValue({ products: [product("p1")] }) })}
        eventSource={factory}
        nowSeconds={() => 1300}
      />,
    );
    const newTask = await screen.findByRole("button", { name: "New task" });
    await waitFor(() => expect(newTask).toBeEnabled());
    await userEvent.click(newTask);
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    await userEvent.click(screen.getByRole("button", { name: "Start task" }));
    const footer = screen.getByRole("contentinfo");
    expect(await within(footer).findByText("Agent started")).toBeInTheDocument();
    await userEvent.click(within(footer).getByRole("button", { name: "History" }));
    expect(within(footer).getByText("could not fetch core; started from the last fetched commit")).toBeInTheDocument();
    localStorage.clear();
  });

  it("closing_the_panel_leaves_fullscreen", async () => {
    render(<App client={fakeClient()} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", goal: "first" }), agent({ id: "b", goal: "second" })] }, "1"));
    await userEvent.click(screen.getByText("first"));
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    await userEvent.click(screen.getByText("second"));
    expect(screen.getByRole("complementary", { name: "Detail" })).toHaveAttribute("data-layout", "side");
  });

  it("escape_closes_the_dialog_and_keeps_fullscreen", async () => {
    localStorage.setItem("fleet.product", "p1");
    render(
      <App
        client={fakeClient({ personas: vi.fn().mockResolvedValue({ personas: [persona("me")] }), products: vi.fn().mockResolvedValue({ products: [product("p1")] }) })}
        eventSource={factory}
        nowSeconds={() => 1300}
      />,
    );
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a", goal: "first", product: "p1" })] }, "1"));
    await userEvent.click(screen.getByText("first"));
    await userEvent.click(screen.getByRole("button", { name: "Fullscreen" }));
    const newTask = screen.getByRole("button", { name: "New task" });
    await waitFor(() => expect(newTask).toBeEnabled());
    await userEvent.click(newTask);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Detail" })).toHaveAttribute("data-layout", "full");
    localStorage.clear();
  });

  it("an_error_stays_until_dismissed_as_lites_default", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
    const run = vi.fn().mockRejectedValue(new Error("agent is not running"));
    render(<App client={fakeClient({ run })} eventSource={factory} nowSeconds={() => 1300} />);
    await screen.findByRole("region", { name: "Working" });
    act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents: [agent({ id: "a" })] }, "1"));
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    const footer = screen.getByRole("contentinfo");
    await within(footer).findByText("agent is not running");
    await act(() => vi.advanceTimersByTimeAsync(20_000));
    expect(within(footer).getByText("agent is not running")).not.toHaveAttribute("data-dimmed");
    vi.useRealTimers();
  });
});
