import { readFileSync } from "node:fs";
import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { InternalBranches } from "../api/generated/InternalBranches";
import type { InternalCommit } from "../api/generated/InternalCommit";
import type { InternalRemote } from "../api/generated/InternalRemote";
import { choose, optionsOf } from "../test/fixtures";
import { click, managedDef as def, managedProduct as product, management, noon, pending, TODAY } from "./harness";

const css = readFileSync("src/management/management.css", "utf8");

// Ports of the Internal resources tests in crates/fleet-gui/tests/management.rs. The fake daemon answers each call
// when the test says, as egui's tests hand the app each AppResult.

type Pending<T> = ReturnType<typeof pending<T>>;

const local: InternalRemote = { url: null, unpushed: 0, error: null, parked: [] };
const branches = (names: string[], remote = local): InternalBranches => ({ branches: names, remote });

/** The app on Products with Alpha and Beta, every internal call held until answered. */
async function internal(over = {}) {
  const calls = {
    branches: [] as [string, Pending<InternalBranches>][],
    files: [] as [string, string, Pending<string[]>][],
    file: [] as [string, string, string, Pending<string>][],
    history: [] as Pending<InternalCommit[]>[],
  };
  const hold =
    <T, A extends unknown[]>(list: [...A, Pending<T>][]) =>
    (...args: A) => {
      const p = pending<T>();
      list.push([...args, p]);
      return p.promise;
    };
  const client = await management("Products", {
    products: [product("p", "Alpha"), product("b", "Beta")],
    over: {
      internalBranches: vi.fn(hold(calls.branches)),
      internalFiles: vi.fn(hold(calls.files)),
      internalFile: vi.fn(hold(calls.file)),
      internalHistory: vi.fn(() => {
        const p = pending<InternalCommit[]>();
        calls.history.push(p);
        return p.promise;
      }),
      ...over,
    },
  });
  await screen.findByRole("heading", { name: "Alpha" });
  return { client, calls };
}

const answer = async <T,>(p: Pending<T>, value: T) => act(async () => p.resolve(value));
const refuse = async (p: { reject: (e: unknown) => void }, why: string) => act(async () => p.reject(new FleetError(why)));
const header = () => screen.getByRole("button", { name: "Internal resources" });
const branch = () => screen.getByRole("combobox", { name: "Internal branch" });

/** egui's `internal_with_files`: opened, with branches `feature` and `main`, then these files on main. */
async function withFiles(files: string[]) {
  const h = await internal();
  await click("Internal resources");
  await answer(h.calls.branches[0]![1], branches(["feature", "main"]));
  await answer(h.calls.files[0]![2], files);
  return h;
}

beforeEach(() => localStorage.clear());

describe("Products: Internal resources", () => {
  it("internal_resources_load_when_the_section_opens_not_before", async () => {
    const { client } = await internal();
    expect(header()).toHaveAttribute("aria-expanded", "false");
    expect(client.internalBranches).not.toHaveBeenCalled();
    await click("Internal resources");
    expect(header()).toHaveAttribute("aria-expanded", "true");
    expect(vi.mocked(client.internalBranches).mock.calls).toEqual([["p"]]);
  });

  it("internal_resources_header_shows_the_remote_and_what_is_unpushed", async () => {
    const { calls } = await internal();
    await click("Internal resources");
    const remote = { url: "https://git.example/notes.git", unpushed: 3, error: "rejected: non-fast-forward", parked: [] };
    await answer(calls.branches[0]![1], branches(["main"], remote));
    for (const text of ["Remote: https://git.example/notes.git", "3 commits not pushed", "rejected: non-fast-forward"])
      expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByText(/parked/)).toBeNull();
  });

  it("internal_resources_header_names_each_parked_branch", async () => {
    const { calls } = await internal();
    await click("Internal resources");
    const parked = ["fleet/unpushed-20261029-101500", "fleet/unpushed-20261029-121500"];
    await answer(calls.branches[0]![1], branches(["main"], { ...local, url: "https://git.example/notes.git", parked }));
    for (const b of parked) expect(screen.getByText(`Unpushed notes parked on ${b}; merge it by hand`)).toBeInTheDocument();
  });

  it("internal_resources_header_says_local_only_without_a_remote", async () => {
    const { calls } = await internal();
    await click("Internal resources");
    await answer(calls.branches[0]![1], branches(["main"]));
    expect(screen.getByText("Local only")).toBeInTheDocument();
    expect(screen.queryByText(/not pushed/)).toBeNull();
  });

  it("internal_resources_list_branches_with_main_first_and_the_files_of_main", async () => {
    const { client } = await withFiles(["designs/a.md", "plans/b.md"]);
    expect(client.internalFiles).toHaveBeenLastCalledWith("p", "main");
    expect(branch()).toHaveTextContent("main");
    expect(await optionsOf("Internal branch")).toEqual(["main", "feature"]);
    for (const name of ["designs/a.md", "plans/b.md"]) expect(screen.getByRole("button", { name })).toBeInTheDocument();
  });

  it("picking_another_branch_lists_its_files", async () => {
    const { client } = await withFiles(["a.md"]);
    await choose("Internal branch", "feature");
    expect(client.internalFiles).toHaveBeenLastCalledWith("p", "feature");
    expect(screen.queryByRole("button", { name: "a.md" })).toBeNull();
  });

  it("selecting_a_file_shows_its_text_and_last_commit", async () => {
    const { client, calls } = await withFiles(["designs/a.md"]);
    await click("designs/a.md");
    expect(client.internalFile).toHaveBeenLastCalledWith("p", "main", "designs/a.md");
    await answer(calls.file[0]![3], "# The design");
    const at = noon(TODAY) - 2 * 3600;
    await answer(calls.history[0]!, [{ sha: "abc", at, author: "orchestrator", subject: "Add the design" }]);
    expect(screen.getByText("# The design")).toBeInTheDocument();
    expect(screen.getByText("Add the design · orchestrator · 2h 0m ago")).toBeInTheDocument();
  });

  it("a_failed_history_shows_the_text_without_the_commit_line", async () => {
    const { calls } = await withFiles(["designs/a.md"]);
    await click("designs/a.md");
    await answer(calls.file[0]![3], "# The design");
    await refuse(calls.history[0]!, "no history");
    expect(screen.getByText("# The design")).toBeInTheDocument();
    expect(screen.queryByText(/ago$/)).toBeNull();
    expect(screen.queryByText("no history")).toBeNull();
  });

  it("a_product_without_a_repo_says_so", async () => {
    const { client, calls } = await internal();
    await click("Internal resources");
    await answer(calls.branches[0]![1], branches([]));
    expect(screen.getByText("No internal resources yet")).toBeInTheDocument();
    expect(client.internalBranches).toHaveBeenCalledTimes(1);
    expect(client.internalFiles).not.toHaveBeenCalled();
  });

  it("an_internal_error_shows_the_daemons_message", async () => {
    const { calls } = await internal();
    await click("Internal resources");
    await refuse(calls.branches[0]![1], "unknown product p");
    expect(screen.getByText("unknown product p")).toBeInTheDocument();
  });

  it("a_refused_file_shows_why", async () => {
    const { calls } = await withFiles(["big.md"]);
    await click("big.md");
    await refuse(calls.file[0]![3], "file is larger than 1 MiB");
    expect(screen.getByText("file is larger than 1 MiB")).toBeInTheDocument();
  });

  it("an_answer_for_another_product_or_branch_is_dropped", async () => {
    const { client, calls } = await internal();
    await click("Internal resources");
    // Alpha's answer lands after Beta is shown: Beta's section is its own, closed.
    await userEvent.click(screen.getByRole("button", { name: "Beta" }));
    await answer(calls.branches[0]![1], branches(["main"]));
    expect(header()).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Local only")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Alpha" }));
    await click("Internal resources");
    await answer(calls.branches[1]![1], branches(["feature", "main"]));
    await choose("Internal branch", "feature");
    // main's files arrive after feature was picked.
    await answer(calls.files[0]![2], ["stale.md"]);
    expect(screen.queryByRole("button", { name: "stale.md" })).toBeNull();
    expect(client.internalFiles).toHaveBeenLastCalledWith("p", "feature");
  });

  it("refresh_refetches_branches_and_keeps_the_chosen_file", async () => {
    const { client, calls } = await withFiles(["a.md"]);
    await click("a.md");
    await answer(calls.file[0]![3], "old text");
    await answer(calls.history[0]!, []);
    await click("Refresh");
    expect(client.internalBranches).toHaveBeenCalledTimes(2);
    await answer(calls.branches[1]![1], branches(["main", "task/new"]));
    expect(client.internalFiles).toHaveBeenLastCalledWith("p", "main");
    await answer(calls.files[1]![2], ["a.md", "b.md"]);
    expect(client.internalFile).toHaveBeenLastCalledWith("p", "main", "a.md");
    await answer(calls.file[1]![3], "new text");
    await answer(calls.history[1]!, []);
    expect(screen.getByRole("button", { name: "b.md" })).toBeInTheDocument();
    expect(screen.getByText("new text")).toBeInTheDocument();
  });

  it("refresh_falls_back_to_main_when_the_chosen_branch_is_gone", async () => {
    const { client, calls } = await withFiles(["a.md"]);
    await choose("Internal branch", "feature");
    await click("Refresh");
    await answer(calls.branches[1]![1], branches(["main"]));
    expect(client.internalFiles).toHaveBeenLastCalledWith("p", "main");
  });

  it("a_failed_branch_fetch_can_be_retried", async () => {
    const { client, calls } = await internal();
    await click("Internal resources");
    await refuse(calls.branches[0]![1], "daemon unreachable");
    expect(screen.getByText("daemon unreachable")).toBeInTheDocument();
    await click("Refresh");
    expect(client.internalBranches).toHaveBeenCalledTimes(2);
    await answer(calls.branches[1]![1], branches([]));
    expect(screen.queryByText("daemon unreachable")).toBeNull();
    expect(screen.getByText("No internal resources yet")).toBeInTheDocument();
  });

  it("reopening_the_section_refetches_branches", async () => {
    const { client, calls } = await internal();
    await click("Internal resources");
    await answer(calls.branches[0]![1], branches([]));
    await click("Internal resources");
    expect(header()).toHaveAttribute("aria-expanded", "false");
    await click("Internal resources");
    expect(vi.mocked(client.internalBranches).mock.calls).toEqual([["p"], ["p"]]);
  });
});

describe("Products: review focus", () => {
  it("the_browser_offers_every_products_action", async () => {
    const { calls } = await internal({ definitions: vi.fn().mockResolvedValue([def("specialist", "rust")]) });
    await click("rust");
    expect(screen.getByRole("button", { name: "Save hidden specialists" })).toBeEnabled();
    for (const name of ["New product", "+ New product", "Delete product", "Add override", "Internal resources"])
      expect(screen.getByRole("button", { name })).toBeEnabled();
    await userEvent.type(screen.getByRole("textbox", { name: "Name" }), "!");
    for (const name of ["Save", "Revert"]) expect(screen.getByRole("button", { name })).toBeEnabled();
    await click("Add override");
    await choose("Override section 1", "limits");
    await choose("Override key 1", "max_steps");
    await userEvent.type(screen.getByRole("textbox", { name: "Override value 1" }), "3");
    expect(screen.getByRole("button", { name: "Save overrides" })).toBeEnabled();
    await click("Internal resources");
    await answer(calls.branches[0]![1], branches([]));
    expect(screen.getByRole("button", { name: "Refresh" })).toBeEnabled();
  });

  it("the_products_css_scrolls_and_uses_tokens_only", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).toMatch(/\.internal-files \{[^}]*max-height: var\(--management-internal-files-height\)[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.internal-text \{[^}]*max-height: var\(--management-internal-text-height\)[^}]*overflow: auto/);
    expect(css).toMatch(/\.product-pane \{[^}]*min-width: 0/);
    // A long remote URL or product name breaks rather than widening the page.
    expect(css).toMatch(/\.management \.note \{[^}]*overflow-wrap: anywhere/);
    expect(css).toMatch(/\.product-pane h2 \{[^}]*overflow-wrap: anywhere/);
    // New product's Save and Cancel sit at the right end, Save rightmost: reversed, so packed from the start.
    expect(css).toMatch(/\.dialog\.new-product \.form-actions\.end \{[^}]*flex-direction: row-reverse;[^}]*justify-content: flex-start;/);
  });
});
