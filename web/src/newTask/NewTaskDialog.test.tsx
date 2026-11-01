import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FleetError } from "../api/client";
import type { CheckoutInfo } from "../api/generated/CheckoutInfo";
import type { Product } from "../api/generated/Product";
import type { RepoInfo } from "../api/generated/RepoInfo";
import { agent, choose, factory, fakeClient, optionsOf, persona, picked, product, repoInfo } from "../test/fixtures";

// Ported from crates/fleet-gui/tests/app.rs (the New task dialog); the names are Lite's.
const release = (id: number, version: string, released_at: number | null = null) => ({ id, product: "p1", version, start: "", target: "", released_at });
const checkout = (path: string, main: boolean, branch: string | null): CheckoutInfo => ({ path, main, branch, commit: "abc1234", subject: "s" });
const started = (id = "new") => ({ agent: agent({ id }), warnings: [] as string[] });

afterEach(() => localStorage.clear());

async function openDialog(over: Parameters<typeof fakeClient>[0] = {}, opts: { repos?: RepoInfo[]; product?: Product } = {}) {
  localStorage.setItem("fleet.product", "p1");
  localStorage.setItem("fleet.persona", "me");
  const client = fakeClient({
    personas: vi.fn().mockResolvedValue({ personas: [persona("me"), persona("work")] }),
    products: vi.fn().mockResolvedValue({ products: [opts.product ?? product("p1")] }),
    repos: vi.fn().mockResolvedValue(opts.repos ?? [repoInfo("core"), repoInfo("portal")]),
    personaDefinitions: vi.fn().mockResolvedValue([]),
    checkouts: vi.fn().mockResolvedValue([]),
    spawn: vi.fn().mockResolvedValue(started()),
    ...over,
  });
  render(<App client={client} eventSource={factory} nowSeconds={() => 1300} />);
  await reopen();
  return { client, dialog: () => within(screen.getByRole("dialog", { name: "New task" })) };
}

async function reopen() {
  const button = await screen.findByRole("button", { name: "New task" });
  await waitFor(() => expect(button).toBeEnabled());
  await userEvent.click(button);
  await screen.findByRole("dialog", { name: "New task" });
}

const startButton = () => screen.getByRole("button", { name: "Start task" });
const pick = (name: string) => userEvent.click(within(screen.getByRole("radiogroup", { name: /^Repo/ })).getByRole("radio", { name }));
const lastSpawn = (client: ReturnType<typeof fakeClient>) => (client.spawn as ReturnType<typeof vi.fn>).mock.calls.at(-1)![0];

describe("New task, as Lite's dialog", () => {
  it("new_task_defaults_to_active_persona", async () => {
    await openDialog();
    expect(await picked("Persona")).toBe("me");
    expect(screen.getByLabelText("Goal")).toHaveFocus();
  });

  it("persona_defaults_to_products_default", async () => {
    await openDialog({}, { product: product("p1", "p1", { default_persona: "work" }) });
    expect(await picked("Persona")).toBe("work");
  });

  it("changing_dialog_persona_reloads_repos", async () => {
    const { client } = await openDialog();
    await waitFor(() => expect(client.repos).toHaveBeenCalledWith("p1"));
    await pick("core");
    const asked = (client.repos as ReturnType<typeof vi.fn>).mock.calls.filter(([p]) => p === "p1").length;
    await choose("Persona", "work");
    await waitFor(() => expect((client.repos as ReturnType<typeof vi.fn>).mock.calls.filter(([p]) => p === "p1").length).toBe(asked + 1));
    expect(within(screen.getByRole("radiogroup", { name: /^Repo/ })).getByRole("radio", { name: "core" }), "the picks go with the persona").toHaveAttribute("aria-checked", "false");
  });

  it("a_failed_repo_load_shows_in_the_dialog", async () => {
    const repos = vi.fn((product?: string) => (product ? Promise.reject(new Error("daemon down")) : Promise.resolve([])));
    const { dialog } = await openDialog({ repos });
    expect(await dialog().findByText("Could not load the repos: daemon down")).toBeInTheDocument();
  });

  it("same_named_personas_stay_apart_in_the_new_task_dialog", async () => {
    const twin = { ...persona("me2"), name: persona("me").name };
    await openDialog({ personas: vi.fn().mockResolvedValue({ personas: [persona("me"), twin] }) });
    const options = await optionsOf("Persona");
    expect(new Set(options).size).toBe(options.length);
    expect(options).toContain(`${twin.name} (me2)`);
  });

  it("new_task_has_no_type_picker", async () => {
    await openDialog();
    expect(screen.queryByRole("combobox", { name: "Type" })).toBeNull();
    expect(screen.queryByText("Solo")).toBeNull();
    const repos = screen.getByRole("radiogroup", { name: "Repo" });
    expect(within(repos).getAllByRole("radio").map((r) => r.textContent), "a task takes exactly one repo").toEqual(["core", "portal"]);
  });

  it("a_new_task_takes_one_repo", async () => {
    await openDialog();
    await pick("core");
    await pick("portal");
    const group = within(screen.getByRole("radiogroup", { name: "Repo" }));
    expect(group.getByRole("radio", { name: "portal" })).toHaveAttribute("aria-checked", "true");
    expect(group.getByRole("radio", { name: "core" }), "the newest pick replaces the old one").toHaveAttribute("aria-checked", "false");
    await pick("portal");
    expect(group.getByRole("radio", { name: "portal" }), "a pick unticks").toHaveAttribute("aria-checked", "false");
  });

  it("the_repo_radios_move_with_the_arrow_keys", async () => {
    await openDialog({}, { repos: [repoInfo("core"), repoInfo("portal"), repoInfo("web")] });
    const group = within(screen.getByRole("radiogroup", { name: "Repo" }));
    const radio = (name: string) => group.getByRole("radio", { name });
    // One stop in the tab order: the first radio while none is chosen, then the chosen one.
    await waitFor(() => expect(radio("core")).toHaveAttribute("tabindex", "0"));
    expect(radio("portal")).toHaveAttribute("tabindex", "-1");
    radio("core").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(radio("portal")).toHaveFocus();
    expect(radio("portal")).toHaveAttribute("aria-checked", "true");
    expect(radio("portal")).toHaveAttribute("tabindex", "0");
    expect(radio("core")).toHaveAttribute("tabindex", "-1");
    await userEvent.keyboard("{ArrowDown}");
    expect(radio("web")).toHaveFocus();
    expect(radio("web")).toHaveAttribute("aria-checked", "true");
    // The ends wrap round.
    await userEvent.keyboard("{ArrowDown}");
    expect(radio("core")).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(radio("web")).toHaveFocus();
    await userEvent.keyboard("{ArrowUp}");
    expect(radio("portal")).toHaveFocus();
    expect(radio("portal")).toHaveAttribute("aria-checked", "true");
    expect(radio("web")).toHaveAttribute("aria-checked", "false");
  });

  it("new_task_needs_a_repo", async () => {
    await openDialog();
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    expect(startButton()).toBeDisabled();
    expect(startButton()).toHaveAttribute("title", "Pick a repo");
  });

  it("new_task_spawns_a_direct_orchestrator", async () => {
    const { client } = await openDialog();
    await pick("core");
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    await userEvent.click(startButton());
    expect(lastSpawn(client)).toMatchObject({ kind: "orchestrator", role: "direct", repos: [{ repo: "core", local: null }] });
  });

  describe("the workflow line", () => {
    const withWorkflow = (workflow: RepoInfo["workflow"]) => [repoInfo("core", { workflow })];
    const withRepos = (repos: RepoInfo[]) => openDialog({}, { repos });

    it("dialog_offers_create_workflow_without_blocking", async () => {
      await withRepos(withWorkflow({ state: "missing", base: "main" }));
      await pick("core");
      expect(screen.getByText(/core has no \.fleet\/workflow\.yml on main\./)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Create workflow" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Create workflow" })).toHaveAttribute("title", "not on the web yet");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      expect(startButton(), "the line never blocks: creation decides").toBeEnabled();
    });

    it("dialog_shows_an_invalid_or_unreadable_workflow", async () => {
      await withRepos(withWorkflow({ state: "invalid", base: "main", error: "bad yaml" }));
      await pick("core");
      expect(screen.getByText("core: .fleet/workflow.yml on main is invalid: bad yaml")).toBeInTheDocument();
    });

    it("dialog_lists_the_found_steps", async () => {
      await withRepos(withWorkflow({ state: "found", base: "main", steps: ["fmt", "test"] }));
      await pick("core");
      expect(screen.getByText("Checks on main: fmt, test")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Create workflow" })).toBeNull();
    });
  });

  describe("where a repo starts", () => {
    const repos = [
      repoInfo("both", { path: "/src/both", url: "https://example.invalid/both.git" }),
      repoInfo("web", { path: null, url: "https://example.invalid/web.git" }),
      repoInfo("tool", { path: "/src/tool", url: null }),
    ];

    it("start_choice_offered_only_with_path_and_url", async () => {
      await openDialog({}, { repos });
      await pick("web");
      expect(screen.queryByRole("button", { name: "From URL" })).toBeNull();
      await pick("both");
      expect(screen.getByRole("button", { name: "From URL" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByRole("button", { name: "Local checkout" })).toHaveAttribute("aria-pressed", "false");
    });

    it("path_only_repo_shows_no_github_hint", async () => {
      const hint = "No URL: no pull request or merge tracking for this repo.";
      await openDialog({ checkouts: vi.fn().mockResolvedValue([checkout("/src/tool", true, "main")]) }, { repos });
      await pick("web");
      expect(screen.queryByText(hint)).toBeNull();
      await pick("tool");
      expect(screen.getByText(hint)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "From URL" }), "no choice without a URL").toBeNull();
    });

    it("spawn_sends_product_release_and_local_start", async () => {
      const checkouts = vi.fn().mockResolvedValue([checkout("/real/both", true, "main"), checkout("/wt/feat", false, "feat")]);
      const p = product("p1", "p1", { releases: [release(7, "1.2.0"), release(8, "1.1.0", 1)] });
      const { client } = await openDialog({ checkouts }, { repos, product: p });
      await choose("Release", "1.2.0");
      expect(await optionsOf("Release")).toEqual(["Backlog", "1.2.0"]);
      await pick("both");
      await userEvent.click(screen.getByRole("button", { name: "Local checkout" }));
      expect(checkouts).toHaveBeenCalledWith("p1", "both");
      await screen.findByRole("combobox", { name: "Checkout of both" });
      expect(await optionsOf("Checkout of both")).toEqual(["main  /real/both", "feat  /wt/feat"]);
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      expect(lastSpawn(client)).toMatchObject({ product: "p1", release: 7, persona: "me", repos: [{ repo: "both", local: "/real/both" }], title: null, kind: "orchestrator", role: "direct", slug: null });
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      await reopen();
      await pick("both");
      await userEvent.click(screen.getByRole("button", { name: "Local checkout" }));
      await screen.findByRole("combobox", { name: "Checkout of both" });
      await choose("Checkout of both", /\/wt\/feat/);
      await userEvent.type(screen.getByLabelText("Goal"), "again");
      await userEvent.click(startButton());
      expect(lastSpawn(client)).toMatchObject({ release: null, repos: [{ repo: "both", local: "/wt/feat" }] });
    });

    it("start_waits_for_a_local_start_to_list_and_stops_on_a_failure", async () => {
      let answer!: (list: CheckoutInfo[]) => void;
      const checkouts = vi
        .fn()
        .mockImplementationOnce(() => new Promise((r) => (answer = r)))
        .mockRejectedValueOnce(new FleetError("not a git repository"))
        .mockResolvedValueOnce([checkout("/src/tool", true, "main")]);
      await openDialog({ checkouts }, { repos });
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await pick("tool");
      expect(screen.getByText("Loading checkouts…")).toBeInTheDocument();
      expect(startButton()).toHaveAttribute("title", "Loading the checkouts of tool…");
      await act(async () => answer([]));
      expect(startButton()).toHaveAttribute("title", "tool has no checkout to start from");
      // Reopening lists again (a branch may have been checked out since): this time it fails, and Retry asks again.
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
      await reopen();
      expect(await screen.findByText("Cannot list the checkouts of tool: not a git repository")).toBeInTheDocument();
      expect(startButton()).toBeDisabled();
      await userEvent.click(screen.getByRole("button", { name: "Retry" }));
      expect(await screen.findByRole("combobox", { name: "Checkout of tool" })).toBeInTheDocument();
      expect(startButton()).toBeEnabled();
    });

    it("a_listing_from_an_earlier_opening_is_dropped", async () => {
      let late!: (list: CheckoutInfo[]) => void;
      const checkouts = vi
        .fn()
        .mockImplementationOnce(() => new Promise((r) => (late = r)))
        .mockResolvedValueOnce([checkout("/src/tool", true, "now")]);
      await openDialog({ checkouts }, { repos });
      await pick("tool");
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
      await reopen();
      expect(await screen.findByRole("combobox", { name: "Checkout of tool" })).toHaveTextContent("now /src/tool");
      await act(async () => late([checkout("/src/tool", true, "stale")]));
      expect(screen.getByRole("combobox", { name: "Checkout of tool" })).toHaveTextContent("now /src/tool");
    });

    it("refetched_repos_clear_the_picked_starts", async () => {
      let done!: (v: ReturnType<typeof started>) => void;
      const { client } = await openDialog(
        { checkouts: vi.fn().mockResolvedValue([checkout("/src/both", true, "main")]), spawn: vi.fn(() => new Promise((r) => (done = r))) },
        { repos },
      );
      await pick("both");
      await userEvent.click(screen.getByRole("button", { name: "Local checkout" }));
      await screen.findByRole("combobox", { name: "Checkout of both" });
      // A spawn lands while the dialog holds a changed goal: it stays open and its repos come again, neither an opening
      // nor a persona switch (which clear the starts themselves), and the start goes back to the URL.
      await userEvent.type(screen.getByLabelText("Goal"), "first");
      await userEvent.click(startButton());
      await userEvent.type(screen.getByLabelText("Goal"), " more");
      const asked = (client.repos as ReturnType<typeof vi.fn>).mock.calls.filter(([p]) => p === "p1").length;
      await act(async () => done(started()));
      await waitFor(() => expect((client.repos as ReturnType<typeof vi.fn>).mock.calls.filter(([p]) => p === "p1").length).toBe(asked + 1));
      await waitFor(() => expect(screen.getByRole("button", { name: "From URL" })).toHaveAttribute("aria-pressed", "true"));
    });

    it("each_start_choice_is_named_by_its_repo", async () => {
      const two = [repoInfo("both", { path: "/src/both", url: "https://example.invalid/both.git" }), repoInfo("also", { path: "/src/also", url: "https://example.invalid/also.git" })];
      await openDialog({}, { repos: two });
      await pick("both");
      expect(screen.getByRole("group", { name: "both starts" })).toBeInTheDocument();
      await pick("also");
      expect(screen.queryByRole("group", { name: "both starts" }), "the newest pick replaces the old one").toBeNull();
      await userEvent.click(within(screen.getByRole("group", { name: "also starts" })).getByRole("button", { name: "Local checkout" }));
      expect(within(screen.getByRole("group", { name: "also starts" })).getByRole("button", { name: "Local checkout" })).toHaveAttribute("aria-pressed", "true");
    });

    it("a_start_the_listing_lacks_shows_its_raw_path", async () => {
      // No main row: the repo's own path is sent, and the select says so rather than showing the first row, as Lite.
      const { client } = await openDialog({ checkouts: vi.fn().mockResolvedValue([checkout("/wt/feat", false, "feat")]) }, { repos });
      await pick("tool");
      expect(await screen.findByRole("combobox", { name: "Checkout of tool" })).toHaveTextContent("/src/tool");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      expect(lastSpawn(client)).toMatchObject({ repos: [{ repo: "tool", local: "/src/tool" }] });
    });
  });

  describe("sending", () => {
    it("spawn_success_clears_the_prompt_and_failure_keeps_it_with_the_error", async () => {
      const spawn = vi.fn().mockRejectedValueOnce(new FleetError('unknown repo "x"')).mockResolvedValueOnce(started());
      await openDialog({ spawn });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "do it");
      await userEvent.type(screen.getByLabelText("Title"), "Short");
      await userEvent.click(startButton());
      expect(await screen.findByText('unknown repo "x"')).toBeInTheDocument();
      expect(screen.getByLabelText("Goal")).toHaveValue("do it");
      expect(spawn.mock.calls[0]![0]).toMatchObject({ title: "Short", prompt: "do it" });
      await userEvent.click(startButton());
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      await reopen();
      expect(screen.getByLabelText("Goal")).toHaveValue("");
      expect(screen.getByLabelText("Title")).toHaveValue("");
    });

    it("a_spawn_refused_for_a_tool_shows_its_fix_and_the_wizard_link", async () => {
      const tool = { kind: "not-installed" as const, tool: "gh" as const, fix: { install: "brew install gh", link: "https://cli.github.com", manager: "brew" } };
      const { client } = await openDialog({ spawn: vi.fn().mockRejectedValue(new FleetError("gh is missing", undefined, tool)) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      expect(await screen.findByText("GitHub CLI (gh) is not installed")).toBeInTheDocument();
      expect(screen.getByText("brew install gh")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "https://cli.github.com" })).toHaveAttribute("href", "https://cli.github.com");
      expect(screen.queryByText("gh is missing"), "the refusal takes the error line's place").toBeNull();
      expect(screen.queryByRole("button", { name: "Install" }), "installing happens only in the wizard").toBeNull();
      await userEvent.click(screen.getByRole("button", { name: "Open the setup wizard" }));
      expect(screen.queryByRole("dialog", { name: "New task" }), "the wizard replaces the dialog").toBeNull();
      expect(await screen.findByRole("heading", { name: /What Fleet Manager needs/ })).toBeInTheDocument();
      expect(client.recheckTools, "the wizard checks the tools").toHaveBeenCalledTimes(1);
    });

    it("a_spawn_refused_for_a_login_does_not_offer_the_wizard", async () => {
      const tool = { kind: "not-logged-in" as const, tool: "claude" as const, persona: "me" };
      await openDialog({ spawn: vi.fn().mockRejectedValue(new FleetError("log in", undefined, tool)) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      expect(await screen.findByText("Claude Code is not logged in for me")).toBeInTheDocument();
      expect(screen.getByText("Log in again from the persona's setup.")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Open the setup wizard" })).toBeNull();
    });

    it("spawn_failure_after_closing_the_dialog_shows_in_the_footer", async () => {
      let fail!: (e: Error) => void;
      await openDialog({ spawn: vi.fn(() => new Promise((_, reject) => (fail = reject))) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      await userEvent.keyboard("{Escape}");
      await act(async () => fail(new FleetError("no such product")));
      expect(await within(screen.getByRole("contentinfo")).findByText("Could not start the agent: no such product")).toBeInTheDocument();
    });

    it("a_successful_spawn_clears_the_chosen_repos", async () => {
      await openDialog();
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      await reopen();
      expect(within(screen.getByRole("radiogroup", { name: /^Repo/ })).getByRole("radio", { name: "core" })).toHaveAttribute("aria-checked", "false");
    });

    it("reopening_drops_the_picks_the_refetched_repos_lack", async () => {
      // Another product's repos by the next opening (the active product changed): a pick they lack has no chip to
      // untick, so it is not sent.
      let opened = 0;
      const repos = vi.fn((p?: string) => Promise.resolve(p === "p1" && ++opened > 1 ? [repoInfo("portal"), repoInfo("web")] : [repoInfo("core"), repoInfo("portal")]));
      const { client } = await openDialog({ repos });
      await pick("core");
      await userEvent.click(screen.getByRole("button", { name: "Close" }));
      await reopen();
      await within(screen.getByRole("radiogroup", { name: /^Repo/ })).findByRole("radio", { name: "web" });
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      expect(startButton()).toHaveAttribute("title", "Pick a repo");
      await pick("portal");
      await userEvent.click(startButton());
      expect(lastSpawn(client)).toMatchObject({ repos: [{ repo: "portal", local: null }] });
    });

    it("reopening_the_dialog_keeps_the_chosen_repos", async () => {
      await openDialog();
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "kept");
      await userEvent.click(screen.getByRole("button", { name: "Close" }));
      await reopen();
      expect(screen.getByLabelText("Goal")).toHaveValue("kept");
      expect(within(screen.getByRole("radiogroup", { name: /^Repo/ })).getByRole("radio", { name: "core" })).toHaveAttribute("aria-checked", "true");
    });

    it("spawn_success_after_closing_does_not_reopen", async () => {
      let done!: (v: ReturnType<typeof started>) => void;
      await openDialog({ spawn: vi.fn(() => new Promise((r) => (done = r))) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      await userEvent.keyboard("{Escape}");
      await act(async () => done(started()));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("spawn_warnings_are_announced_after_the_success", async () => {
      await openDialog({ spawn: vi.fn().mockResolvedValue({ agent: agent({ id: "a9" }), warnings: ["could not fetch core"] }) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      const footer = within(screen.getByRole("contentinfo"));
      expect(await footer.findByText("Agent started")).toBeInTheDocument();
      await userEvent.click(footer.getByRole("button", { name: "History" }));
      expect(footer.getByText("could not fetch core")).toBeInTheDocument();
    });

    it("spawn_success_keeps_a_dialog_reopened_and_typed_into_while_in_flight", async () => {
      let done!: (v: ReturnType<typeof started>) => void;
      await openDialog({ spawn: vi.fn(() => new Promise((r) => (done = r))) });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "first");
      await userEvent.click(startButton());
      await userEvent.keyboard("{Escape}");
      await reopen();
      await userEvent.type(screen.getByLabelText("Goal"), " and more");
      await act(async () => done(started()));
      expect(screen.getByRole("dialog", { name: "New task" })).toBeInTheDocument();
      expect(screen.getByLabelText("Goal")).toHaveValue("first and more");
    });

    it("ctrl_enter_starts_the_task", async () => {
      const { client } = await openDialog();
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go{Control>}{Enter}{/Control}");
      expect(lastSpawn(client)).toMatchObject({ prompt: "go" });
    });

    it("a_second_start_while_one_is_on_its_way_is_not_sent", async () => {
      const spawn = vi.fn(() => new Promise(() => {}));
      await openDialog({ spawn });
      await pick("core");
      await userEvent.type(screen.getByLabelText("Goal"), "go");
      await userEvent.click(startButton());
      expect(startButton()).toBeDisabled();
      await userEvent.type(screen.getByLabelText("Goal"), "{Control>}{Enter}{/Control}");
      expect(spawn).toHaveBeenCalledTimes(1);
    });
  });

  it("a_saved_product_is_enough_when_the_products_cannot_load", async () => {
    const { client } = await openDialog({ products: vi.fn().mockRejectedValue(new Error("down")) });
    await waitFor(() => expect(client.repos).toHaveBeenCalledWith("p1"));
    await pick("core");
    await userEvent.type(screen.getByLabelText("Goal"), "go");
    await userEvent.click(startButton());
    expect(lastSpawn(client)).toMatchObject({ product: "p1", repos: [{ repo: "core", local: null }], release: null });
  });

  it("opening_and_a_spawn_ask_for_the_repos_again_as_lites_refresh_repos", async () => {
    let done!: (v: ReturnType<typeof started>) => void;
    const { client } = await openDialog({ spawn: vi.fn(() => new Promise((r) => (done = r))) });
    const repos = client.repos as ReturnType<typeof vi.fn>;
    const board = () => repos.mock.calls.filter(([p]) => p === undefined).length;
    const dialog = () => repos.mock.calls.filter(([p]) => p === "p1").length;
    await waitFor(() => expect(board(), "the board's list, at load and on opening").toBe(2));
    await pick("core");
    await userEvent.type(screen.getByLabelText("Goal"), "first");
    await userEvent.click(startButton());
    await userEvent.type(screen.getByLabelText("Goal"), " more");
    const before = dialog();
    await act(async () => done(started()));
    await waitFor(() => expect(dialog(), "the dialog stayed open: its repos come again").toBe(before + 1));
  });

  it("new_task_css_uses_tokens_only", () => {
    const css = readFileSync("src/newTask/newTask.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });
});
