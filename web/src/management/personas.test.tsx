import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { Persona } from "../api/generated/Persona";
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
