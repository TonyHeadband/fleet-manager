import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { DefInfo } from "../api/generated/DefInfo";
import type { Persona } from "../api/generated/Persona";
import { fakeNative } from "../test/fixtures";
import { CHECK_MS } from "../startup/PersonaSetup";
import { click, login, managedPersona as persona, management, pending } from "./harness";

// Ports of the Personas tests in crates/fleet-gui/tests/management.rs.

const personas = (list: Persona[], over = {}) => management("Personas", { personas: list, over });
const row = (name: string) => screen.findByRole("button", { name });
const field = (name: string) => screen.getByRole("textbox", { name });

beforeEach(() => localStorage.clear());
afterEach(() => vi.useRealTimers());

describe("Personas", () => {
  it("personas_tab_lists_states", async () => {
    await personas([persona("a", "Ready", true), persona("b", "Broken", true, { claude: login(false) }), persona("c", "Draft", false)]);
    expect(await row("Ready ✓")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Broken ⚠" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Draft draft" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "+ New persona" })).toBeInTheDocument();
  });

  it("draft_shows_resume_and_delete_and_resume_opens_the_login_step", async () => {
    await personas([persona("a", "Ready", true), persona("c", "Draft", false)]);
    await userEvent.click(await row("Draft draft"));
    expect(screen.getByText("Setup was not finished.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
    await click("Resume");
    expect(within(screen.getByRole("dialog")).getByRole("heading", { name: "Log in" })).toBeInTheDocument();
    expect(screen.queryByText(/Step 4 of 4/)).toBeNull();
  });

  it("selecting_a_persona_checks_it_and_shows_its_pane", async () => {
    const client = await personas([persona("a", "Work", true), persona("b", "Home", true)]);
    await waitFor(() => expect(client.checkPersona).toHaveBeenCalledWith("a"));
    await userEvent.click(await row("Home ✓"));
    expect(field("Name")).toHaveValue("Home");
  });

  it("delete_disabled_with_tasks", async () => {
    await personas([persona("a", "Work", true, { tasks: 2 })]);
    await row("Work ✓");
    const del = screen.getByRole("button", { name: "Delete" });
    expect(del).toBeDisabled();
    expect(del).toHaveAttribute("title", "Delete or archive its tasks first");
    expect(screen.getByText("Used by 2 tasks")).toBeInTheDocument();
  });

  it("delete_asks_in_the_app_then_deletes", async () => {
    const list = [persona("a", "Work", true), persona("b", "Home", true)];
    const left = vi
      .fn()
      .mockResolvedValueOnce({ personas: list })
      .mockResolvedValue({ personas: [list[1]] });
    const client = await personas(list, { personas: left });
    await row("Work ✓");
    await click("Delete");
    const dialog = screen.getByRole("dialog", { name: "Delete Work?" });
    expect(
      within(dialog).getByText(
        "Its folder is removed, with its Claude and GitHub logins, and so are its clones of product repos. The repos stay with their products. Clones from before products, and clones holding branches its archived tasks kept, stay where they are.",
      ),
    ).toBeInTheDocument();
    expect(client.deletePersona).not.toHaveBeenCalled();
    // Cancel changes nothing.
    await click("Cancel");
    expect(screen.queryByRole("dialog")).toBeNull();
    await click("Delete");
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Delete" }));
    expect(client.deletePersona).toHaveBeenCalledWith("a");
    await waitFor(() => expect(screen.queryByRole("button", { name: "Work ✓" })).toBeNull());
    expect(screen.getByRole("button", { name: "Home ✓" })).toBeInTheDocument();
  });

  it("new_persona_opens_setup_without_step_numbers", async () => {
    await personas([persona("a", "Work", true)]);
    await click("+ New persona");
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: "New persona" })).toBeInTheDocument();
    expect(screen.queryByText(/Step /)).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("leaving_management_forgets_the_delete_dialog", async () => {
    await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    await click("Delete");
    expect(screen.getByRole("dialog", { name: "Delete Work?" })).toBeInTheDocument();
    await click("Kanban");
    await click("Management");
    expect(screen.queryByRole("dialog", { name: "Delete Work?" })).toBeNull();
  });

  it("personas_tab_has_no_repos_section", async () => {
    await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    expect(screen.queryByText(/Remembered repos/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Save hidden specialists" })).toBeNull();
  });

  it("the_detail_pane_sits_inside_the_window", async () => {
    await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    const detail = screen.getByRole("region", { name: "Work" });
    for (const name of ["Save identity", "Delete"]) expect(within(detail).getByRole("button", { name })).toBeInTheDocument();
    expect(within(detail).queryByRole("button", { name: "+ New persona" })).toBeNull();
    expect(screen.getByRole("button", { name: "+ New persona" }).closest(".list-detail-list")).not.toBeNull();
  });

  it("checks_repeat_only_while_a_login_is_not_ok_and_stop_when_the_tab_is_left", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const broken = persona("b", "Broken", true, { github: login(false) });
    const client = await personas([broken, persona("a", "Work", true)]);
    await row("Broken ⚠");
    const calls = () => vi.mocked(client.checkPersona).mock.calls.length;
    const first = calls();
    await act(async () => vi.advanceTimersByTime(CHECK_MS));
    expect(calls()).toBe(first + 1);
    // A settled persona is not asked again.
    await userEvent.click(screen.getByRole("button", { name: "Work ✓" }));
    const settled = calls();
    await act(async () => vi.advanceTimersByTime(3 * CHECK_MS));
    expect(calls()).toBe(settled);
    // Leaving the tab stops the broken one's checks.
    await userEvent.click(screen.getByRole("button", { name: "Broken ⚠" }));
    await click("Overview");
    const left = calls();
    await act(async () => vi.advanceTimersByTime(3 * CHECK_MS));
    expect(calls()).toBe(left);
  });

  it("deleting_the_selected_persona_selects_the_first_left", async () => {
    const list = [persona("a", "Work", true), persona("b", "Home", true), persona("c", "Side", true)];
    const left = vi
      .fn()
      .mockResolvedValueOnce({ personas: list })
      .mockResolvedValue({ personas: [list[0], list[2]] });
    await personas(list, { personas: left });
    await userEvent.click(await row("Home ✓"));
    await click("Delete");
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(field("Name")).toHaveValue("Work"));
  });

  it("the_tab_says_loading_until_the_personas_answer", async () => {
    const answer = pending<{ personas: Persona[] }>();
    await management("Personas", { personas: [persona("a", "Work", true)], over: { personas: vi.fn(() => answer.promise) } });
    expect(screen.getByText("Loading…")).toBeInTheDocument();
    await act(async () => answer.resolve({ personas: [persona("a", "Work", true)] }));
    expect(await row("Work ✓")).toBeInTheDocument();
  });

  it("cancel_on_log_in_deletes_a_new_draft_and_keeps_a_resumed_one", async () => {
    const draft = persona("c", "Draft", false);
    const client = await personas([persona("a", "Work", true), draft], {
      createPersona: vi.fn().mockResolvedValue(persona("n", "New", false)),
    });
    // Resumed: Cancel only closes; the draft has its own Delete.
    await userEvent.click(await row("Draft draft"));
    await click("Resume");
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(client.deletePersona).not.toHaveBeenCalled();
    // New: Cancel on Log in deletes the draft it created.
    await click("+ New persona");
    for (const [label, text] of [
      ["Name", "New"],
      ["Commit name", "Ann"],
      ["Commit email", "a@x.invalid"],
    ] as const)
      await userEvent.type(within(screen.getByRole("dialog")).getByRole("textbox", { name: label }), text);
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Next" }));
    await userEvent.click(await within(screen.getByRole("dialog")).findByRole("button", { name: "Cancel" }));
    expect(client.deletePersona).toHaveBeenCalledWith("n");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("escape_closes_the_setup_outside_the_wizard", async () => {
    await personas([persona("a", "Work", true)]);
    await click("+ New persona");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("a_failed_check_says_so_for_the_selected_persona", async () => {
    await personas([persona("a", "Work", true)], { checkPersona: vi.fn().mockRejectedValue(new FleetError("daemon down")) });
    expect(await screen.findByText("Could not check the logins: daemon down")).toBeInTheDocument();
  });
});

const none = { name: null, commit_name: null, commit_email: null, github_auth: null, nicknames: null };
const specialist = (name: string, error: string | null = null): DefInfo => ({
  kind: "specialist",
  name,
  summary: "",
  builtin: true,
  overridden: false,
  error,
});

describe("Persona edits and logins", () => {
  it("a_nickname_field_is_egui_s_width", () => {
    const css = readFileSync("src/management/management.css", "utf8");
    expect(css).toMatch(/\.nicknames input \{[^}]*width: var\(--management-nickname-width\)/);
  });

  it("rename_sends_only_the_name", async () => {
    const client = await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    await userEvent.type(field("Name"), " 2");
    await click("Save name");
    expect(client.updatePersona).toHaveBeenCalledWith("a", { ...none, name: "Work 2" });
  });

  it("the_identity_needs_both_fields_and_a_change", async () => {
    const client = await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    const save = () => screen.getByRole("button", { name: "Save identity" });
    expect(save()).toBeDisabled();
    await userEvent.clear(field("Commit email"));
    expect(save()).toBeDisabled();
    await userEvent.type(field("Commit email"), "ann@work.invalid");
    expect(save()).toBeEnabled();
    await userEvent.click(save());
    expect(client.updatePersona).toHaveBeenCalledWith("a", { ...none, commit_name: "Ann Example", commit_email: "ann@work.invalid" });
  });

  it("nicknames_are_saved_on_the_persona", async () => {
    const saved = persona("a", "Work", true, { nicknames: { rust: "Ferris" } });
    const updatePersona = vi.fn().mockResolvedValue(saved);
    const definitions = vi
      .fn()
      .mockResolvedValue([specialist("docs"), specialist("rust"), specialist("stale", "specialists/stale.md: bad")]);
    const client = await personas([persona("a", "Work", true)], { updatePersona, definitions });
    // Only specialists that load get a field.
    expect(await screen.findByRole("textbox", { name: "rust" })).toHaveAttribute("placeholder", "none");
    expect(screen.queryByRole("textbox", { name: "stale" })).toBeNull();
    const save = () => screen.getByRole("button", { name: "Save nicknames" });
    expect(save()).toBeDisabled();
    await userEvent.type(field("rust"), " Ferris ");
    await userEvent.click(save());
    expect(client.updatePersona).toHaveBeenCalledWith("a", { ...none, nicknames: { rust: "Ferris" } });
    await waitFor(() => expect(save()).toBeDisabled());
  });

  it("relogin_switch_to_pat_sends_update", async () => {
    const switched = persona("a", "Work", true, { github_auth: "pat" });
    const client = await personas([persona("a", "Work", true)], { updatePersona: vi.fn().mockResolvedValue(switched) });
    await row("Work ✓");
    await click("Personal access token");
    expect(screen.getByText("Re-login switches to this.")).toBeInTheDocument();
    await click("Re-login to GitHub");
    expect(client.updatePersona).toHaveBeenCalledWith("a", { ...none, github_auth: "pat" });
    expect(await screen.findByRole("button", { name: "Save token" })).toBeInTheDocument();
  });

  it("pat_relogin_uses_the_masked_field_and_records_no_token", async () => {
    const native = fakeNative();
    const client = await management("Personas", { personas: [persona("a", "Work", true, { github_auth: "pat" })], native, local: true });
    await row("Work ✓");
    await click("Re-login to GitHub");
    // No auth switch: no update, and the token field opens.
    expect(client.updatePersona).not.toHaveBeenCalled();
    const token = screen.getByLabelText("Personal access token");
    expect(token).toHaveAttribute("type", "password");
    await userEvent.type(token, "ghp_secret");
    await click("Save token");
    expect(native.savePat).toHaveBeenCalledWith("a", "ghp_secret");
    // The token left the field with the call.
    expect(screen.queryByDisplayValue("ghp_secret")).toBeNull();
  });

  it("a_login_result_for_another_persona_is_dropped", async () => {
    const opened = pending<{ script: string | null }>();
    const native = fakeNative({ runLogin: vi.fn(() => opened.promise) });
    await management("Personas", { personas: [persona("a", "Work", true), persona("b", "Home", true)], native, local: true });
    await row("Work ✓");
    await click("Re-login to Claude");
    await click("Home ✓");
    // Work's terminal answers after the switch to Home.
    await act(async () => opened.resolve({ script: "claude /login" }));
    expect(screen.queryByText(/No terminal found/)).toBeNull();
  });

  it("a_token_result_for_another_persona_is_dropped", async () => {
    const saved = pending<void>();
    const native = fakeNative({ savePat: vi.fn(() => saved.promise) });
    const pat = persona("a", "Work", true, { github_auth: "pat" });
    await management("Personas", { personas: [pat, persona("b", "Home", true)], native, local: true });
    await row("Work ✓");
    await click("Re-login to GitHub");
    await userEvent.type(screen.getByLabelText("Personal access token"), "x");
    await click("Save token");
    await click("Home ✓");
    await act(async () => saved.reject(new Error("bad token")));
    expect(screen.queryByText(/bad token/)).toBeNull();
  });

  it("the_two_relogins_are_named", async () => {
    await personas([persona("a", "Work", true)]);
    await row("Work ✓");
    expect(screen.getByRole("button", { name: "Re-login to Claude" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Re-login to GitHub" })).toBeInTheDocument();
  });

  it("in_a_browser_relogin_shows_the_script_and_the_token_waits_for_the_app", async () => {
    await personas([persona("a", "Work", true), persona("p", "Token", true, { github_auth: "pat" })]);
    await row("Work ✓");
    await click("Re-login to Claude");
    expect(await screen.findByText("No terminal found. Run this in one:")).toBeInTheDocument();
    expect(screen.getByText(/claude/)).toBeInTheDocument();
    await click("Token ✓");
    await click("Re-login to GitHub");
    const save = screen.getByRole("button", { name: "Save token" });
    expect(save).toBeDisabled();
    expect(save).toHaveAttribute("title", "Only the Fleet Manager app on the daemon's machine can hand a token to gh");
  });

  it("a_refused_save_or_delete_says_why", async () => {
    const client = await personas([persona("a", "Work", true)], {
      updatePersona: vi.fn().mockRejectedValue(new FleetError("name taken")),
      deletePersona: vi.fn().mockRejectedValue(new FleetError("in use")),
    });
    await row("Work ✓");
    await userEvent.type(field("Name"), " 2");
    await click("Save name");
    expect(await screen.findByText("Could not save: name taken")).toBeInTheDocument();
    expect(field("Name")).toHaveValue("Work 2");
    const asked = vi.mocked(client.personas).mock.calls.length;
    await click("Delete");
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Could not delete the persona: in use")).toBeInTheDocument();
    // The list is asked for again: the daemon's says what is left.
    expect(client.personas).toHaveBeenCalledTimes(asked + 1);
  });
});
