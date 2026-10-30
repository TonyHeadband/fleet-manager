import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { DefFile } from "../api/generated/DefFile";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import { click, management, pending } from "./harness";

// Ports of the definitions tests in crates/fleet-gui/tests/management.rs.

const css = readFileSync("src/management/management.css", "utf8");

/** egui's `def`: a built-in of `kind`, with why it cannot be used. */
const def = (kind: DefKind, name: string, error: string | null = null, over: Partial<DefInfo> = {}): DefInfo => ({
  kind,
  name,
  summary: "",
  builtin: true,
  overridden: false,
  model: null,
  error,
  mode: null,
  effort: null,
  ...over,
});

const file = (kind: DefKind, name: string, text: string, builtin: boolean): DefFile => ({ kind, name, text, builtin });

/** Management on `tab` with these definitions; reads, saves and resets wait to be answered by hand. */
async function defsTab(tab: string, defs: DefInfo[]) {
  const reads: ReturnType<typeof pending<DefFile>>[] = [];
  const saves: ReturnType<typeof pending<DefInfo>>[] = [];
  const resets: ReturnType<typeof pending<void>>[] = [];
  const answered = <T,>(list: ReturnType<typeof pending<T>>[]) =>
    vi.fn(() => {
      const p = pending<T>();
      list.push(p);
      return p.promise;
    });
  const over = {
    definitions: vi.fn().mockResolvedValue(defs),
    definition: answered(reads),
    saveDefinition: answered(saves),
    resetDefinition: answered(resets),
  };
  await management(tab, { over });
  return { ...over, reads, saves, resets };
}

const row = (name: string) => screen.findByRole("button", { name });
const footer = () => screen.getByRole("contentinfo");
const editor = (name: string) => screen.getByRole("textbox", { name });
const land = async <T,>(p: ReturnType<typeof pending<T>>, v: T) => act(async () => p.resolve(v));
const refuse = async <T,>(p: ReturnType<typeof pending<T>>, message: string) => act(async () => p.reject(new FleetError(message)));

beforeEach(() => localStorage.clear());

describe("Definitions", () => {
  it("definitions_tabs", async () => {
    const h = await defsTab("Agent settings", [
      def("orchestrator", "bug-fix"),
      def("specialist", "rust", null, { overridden: true }),
      def("specialist", "go", null, { builtin: false }),
      def("specialist", "broken", "specialists/broken.md: no skills"),
    ]);
    for (const tab of ["Orchestrators", "Specialists", "Skills"]) expect(screen.getByRole("tab", { name: tab })).toBeInTheDocument();

    // A built-in is read-only until Override.
    await click("Orchestrators");
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Orchestrators");
    await userEvent.click(await row("bug-fix built-in"));
    expect(screen.queryByRole("button", { name: /rust/ })).toBeNull();
    expect(h.definition).toHaveBeenLastCalledWith("orchestrator", "bug-fix");
    const text = "---\nname: bug-fix\n---\nFix it.";
    await land(h.reads[0]!, file("orchestrator", "bug-fix", text, true));
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    // egui's `caption`, drawn low like every other.
    expect(screen.getByText("orchestrators/bug-fix")).toHaveClass("caption");
    await userEvent.type(editor("orchestrators/bug-fix"), "typed");
    expect(editor("orchestrators/bug-fix")).toHaveValue(text);
    await click("Override");
    await userEvent.type(editor("orchestrators/bug-fix"), " Now.");
    await click("Save");
    expect(h.saveDefinition).toHaveBeenLastCalledWith("orchestrator", "bug-fix", `${text} Now.`);

    // An invalid file: the daemon's message under the editor, and the draft kept.
    await refuse(h.saves[0]!, "orchestrators/bug-fix.md: missing `skills`");
    expect(screen.getByText(/missing `skills`/)).toBeInTheDocument();
    expect(editor("orchestrators/bug-fix")).toHaveValue(`${text} Now.`);

    // The list says what each one is; Reset asks first.
    await click("Specialists");
    expect(await row("go custom")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "broken built-in ⚠" })).toBeInTheDocument();
    expect(screen.getByText(/no skills/)).toBeInTheDocument();
    await click("rust overrides built-in");
    await land(h.reads[1]!, file("specialist", "rust", "---\nname: rust\n---\nMine.", false));
    expect(screen.getByRole("button", { name: "Revert" })).toBeInTheDocument();
    await click("Reset to built-in");
    expect(screen.getByRole("dialog", { name: "Reset rust to built-in?" })).toBeInTheDocument();
    expect(h.resetDefinition).not.toHaveBeenCalled();
    await click("Cancel");
    await click("Reset to built-in");
    await click("Reset");
    expect(h.resetDefinition).toHaveBeenLastCalledWith("specialist", "rust");
    // A refused reset says why under the editor, like a refused save, and keeps the file.
    await refuse(h.resets[0]!, "specialists/rust.md: permission denied");
    expect(screen.getByText(/permission denied/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset to built-in" })).toBeInTheDocument();

    await click("Reset to built-in");
    await click("Reset");
    await land(h.resets[1]!, undefined);
    // egui's toast, in the footer.
    expect(within(footer()).getByText("rust reset to built-in")).toBeInTheDocument();
    // The draft is gone and the built-in is read again.
    expect(h.definition).toHaveBeenLastCalledWith("specialist", "rust");
    expect(screen.queryByRole("textbox")).toBeNull();
    const builtin = "---\nname: rust\n---\nBuilt in.";
    await land(h.reads[2]!, file("specialist", "rust", builtin, true));
    expect(screen.getByRole("button", { name: "Override" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(editor("specialists/rust")).toHaveValue(builtin);
  });

  it("pr_styles_tab_lists_and_edits_styles", async () => {
    const names = ["conventional-semver-pre1.0", "conventional-semver", "conventional", "minimal", "plain"];
    await defsTab("PR styles", [...names.map((n) => def("pr_style", n)), def("pr_style", "broken", "pr_styles/broken.md: no title")]);
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("PR styles");
    for (const name of names) expect(await row(`${name} built-in`)).toBeInTheDocument();
  });

  it("an_empty_kind_says_so_and_nothing_is_selected", async () => {
    await defsTab("Skills", [def("orchestrator", "bug-fix")]);
    expect(await screen.findByText("None yet.")).toBeInTheDocument();
    expect(screen.getByText("No definition selected.")).toBeInTheDocument();
  });

  it("a_saved_definition_is_the_users_file_and_the_list_is_asked_again", async () => {
    const h = await defsTab("Orchestrators", [def("orchestrator", "bug-fix", null, { mode: "auto", effort: "high" })]);
    // egui's `mode_effort` line under the name.
    expect(await screen.findByText("auto · high")).toBeInTheDocument();
    await click("bug-fix built-in");
    await land(h.reads[0]!, file("orchestrator", "bug-fix", "a", true));
    await click("Override");
    await userEvent.type(editor("orchestrators/bug-fix"), "b");
    await click("Save");
    // One save at a time.
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await land(h.saves[0]!, def("orchestrator", "bug-fix", null, { builtin: false, overridden: true }));
    expect(within(footer()).getByText("bug-fix saved")).toBeInTheDocument();
    expect(h.definitions).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Revert" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Override" })).toBeNull();
  });

  it("a_draft_outlives_the_tab_and_the_view", async () => {
    const h = await defsTab("Specialists", [def("specialist", "go", null, { builtin: false })]);
    await click("go custom");
    await land(h.reads[0]!, file("specialist", "go", "mine", false));
    await userEvent.type(editor("specialists/go"), "!");
    await click("Orchestrators");
    expect(screen.getByText("No definition selected.")).toBeInTheDocument();
    await click("Kanban");
    await click("Management");
    await click("Specialists");
    expect(editor("specialists/go")).toHaveValue("mine!");
    await click("Revert");
    expect(editor("specialists/go")).toHaveValue("mine");
  });

  it("a_failed_list_load_tells_the_footer", async () => {
    await management("Orchestrators", { over: { definitions: vi.fn().mockRejectedValue(new FleetError("daemon down")) } });
    expect(await within(footer()).findByText("Could not load the orchestrators and specialists: daemon down")).toBeInTheDocument();
    // The tab waits for a list, as egui's.
    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });

  it("a_failed_read_says_why", async () => {
    const h = await defsTab("Skills", [def("skill", "review", null, { builtin: false })]);
    await click("review custom");
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    await refuse(h.reads[0]!, "skills/review.md: unreadable");
    expect(screen.getByText("skills/review.md: unreadable")).toBeInTheDocument();
  });

  it("the_management_css_scrolls_and_uses_tokens_only", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(readFileSync("src/ui/ui.css", "utf8")).toMatch(/\.tabbed-page \{[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.definition-editor textarea \{[^}]*font-family: var\(--font-mono\)/);
  });

  it("the_browser_offers_every_management_action", async () => {
    const h = await defsTab("Specialists", [def("specialist", "rust", null, { overridden: true }), def("specialist", "go")]);
    await click("go built-in");
    await land(h.reads[0]!, file("specialist", "go", "built", true));
    expect(screen.getByRole("button", { name: "Override" })).toBeEnabled();
    await click("Override");
    for (const name of ["Save", "Revert"]) expect(screen.getByRole("button", { name })).toBeEnabled();
    await click("rust overrides built-in");
    await land(h.reads[1]!, file("specialist", "rust", "mine", false));
    expect(screen.getByRole("button", { name: "Reset to built-in" })).toBeEnabled();
    await click("Agent settings");
    for (const name of ["Add allow rule", "Add deny rule", "Add variable"])
      expect(await screen.findByRole("button", { name })).toBeEnabled();
  });
});
