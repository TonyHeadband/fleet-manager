import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";
import { FleetError } from "../api/client";
import type { DefFile } from "../api/generated/DefFile";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import { fakeClient } from "../test/fixtures";
import { useDefinitions } from "./definitions";
import { Definitions } from "./Definitions";
import { SpecialistForm } from "./SpecialistForm";

const TEXT = [
  "---",
  "name: rust",
  "summary: Rust code",
  "skills: [test-driven-development]",
  "model: sonnet",
  "mode: auto",
  "effort: medium",
  "extra: kept",
  "---",
  "# You are a Rust specialist",
  "",
  "Write tests first.",
  "",
].join("\n");

const info = (over: Partial<DefInfo> = {}): DefInfo => ({
  kind: "specialist",
  name: "rust",
  summary: "Rust code",
  builtin: true,
  overridden: false,
  error: null,
  nickname: null,
  ...over,
});

const SKILLS = ["test-driven-development", "systematic-debugging", "verification-before-completion"];

function Host(p: {
  client: ReturnType<typeof fakeClient>;
  kind?: DefKind;
  info: DefInfo;
  skills?: string[];
  persona?: string;
  name?: string;
  onNick?: () => void;
  roleFixed?: boolean;
}) {
  const persona = p.persona ?? "a";
  const name = p.name ?? "rust";
  const defs = useDefinitions(p.client, persona, vi.fn(), p.onNick);
  // A move reopens the definition under its new kind.
  const picked = defs.st.selected?.kind;
  const kind = (picked === "orchestrator" || picked === "specialist" ? picked : (p.kind ?? "specialist")) as "specialist" | "orchestrator";
  useEffect(() => {
    defs.loadList();
    defs.open(kind, name);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [persona, name]);
  const listed = defs.st.list?.find((d) => d.name === name) ?? { ...p.info, name };
  return <SpecialistForm persona={persona} kind={kind} name={name} info={listed} defs={defs} skills={p.skills ?? SKILLS} roleFixed={p.roleFixed ?? true} />;
}

async function form(file: Partial<DefFile> = {}, over: Partial<DefInfo> = {}, clientOver: Record<string, unknown> = {}, host: Partial<Parameters<typeof Host>[0]> = {}) {
  const client = fakeClient({
    personaDefinition: vi.fn(async (_p: string, kind: DefKind, name: string) => ({ kind, name, text: TEXT, builtin: true, ...file })),
    personaDefinitions: vi.fn().mockResolvedValue([info(over)]),
    ...clientOver,
  });
  const ui = (h: Partial<Parameters<typeof Host>[0]> = {}) => <Host client={client} info={info(over)} {...host} {...h} />;
  const view = render(ui());
  await screen.findByText("Summary");
  return Object.assign(client, { view, ui });
}

const field = (name: string) => screen.getByRole("textbox", { name });
const button = (name: string) => screen.getByRole("button", { name });
const ownCopy = { builtin: false };

describe("SpecialistForm", () => {
  it("the_form_shows_the_files_fields_and_body", async () => {
    await form(ownCopy, { builtin: false, overridden: true, nickname: "Ferris" });
    expect(field("Name")).toHaveValue("rust");
    expect(field("Name")).toHaveAttribute("readonly");
    expect(field("Nickname")).toHaveValue("Ferris");
    expect(screen.getByText("Specialist")).toBeInTheDocument();
    expect(field("Summary")).toHaveValue("Rust code");
    expect(button("test-driven-development ×")).toBeInTheDocument();
    expect(field("Model")).toHaveValue("sonnet");
    expect(screen.getByRole("combobox", { name: "Mode" })).toHaveTextContent("Auto");
    expect(button("Medium")).toHaveAttribute("aria-pressed", "true");
    expect(field("Instructions")).toHaveValue("# You are a Rust specialist\n\nWrite tests first.\n");
  });

  it("a_builtin_is_read_only_until_override", async () => {
    await form();
    expect(field("Summary")).toHaveAttribute("readonly");
    expect(field("Model")).toHaveAttribute("readonly");
    expect(field("Instructions")).toHaveAttribute("readonly");
    expect(button("Save")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Reset to built-in" })).toBeNull();
    await userEvent.click(button("Override"));
    expect(field("Summary")).not.toHaveAttribute("readonly");
    expect(screen.queryByRole("button", { name: "Override" })).toBeNull();
    // An override not saved yet is a change in itself.
    expect(button("Save")).toBeEnabled();
  });

  it("nickname_on_a_builtin_saves_no_override", async () => {
    const client = await form();
    await userEvent.type(field("Nickname"), " Ferris ");
    await userEvent.click(button("Save"));
    expect(client.setNickname).toHaveBeenCalledWith("a", "rust", "Ferris");
    expect(client.savePersonaDefinition).not.toHaveBeenCalled();
    await waitFor(() => expect(client.personaDefinitions).toHaveBeenCalledTimes(2));
  });

  it("editing_model_saves_the_file_with_only_that_line_changed", async () => {
    const client = await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.tripleClick(field("Model"));
    await userEvent.keyboard("opus");
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust", TEXT.replace("model: sonnet", "model: opus"));
    expect(client.setNickname).not.toHaveBeenCalled();
  });

  it("skills_add_from_the_personas_skills_and_remove_by_chip", async () => {
    const client = await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.click(screen.getByRole("combobox", { name: "Add skill" }));
    expect(screen.queryByRole("option", { name: "test-driven-development" })).toBeNull();
    await userEvent.click(screen.getByRole("option", { name: "systematic-debugging" }));
    expect(button("systematic-debugging ×")).toBeInTheDocument();
    await userEvent.click(button("test-driven-development ×"));
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith(
      "a",
      "specialist",
      "rust",
      TEXT.replace("skills: [test-driven-development]", "skills: [systematic-debugging]"),
    );
  });

  it("a_refused_skill_shows_under_skills", async () => {
    const message = "specialists/rust.md: skill `ghost` not found";
    const savePersonaDefinition = vi.fn().mockRejectedValue(new FleetError(message));
    await form(ownCopy, { builtin: false, overridden: true }, { savePersonaDefinition });
    await userEvent.type(field("Summary"), "!");
    await userEvent.click(button("Save"));
    const shown = await screen.findByText(message);
    expect(shown).toHaveClass("error-line");
    expect(screen.getByText("Skills").closest("div")).toContainElement(shown);
    expect(field("Summary")).toHaveValue("Rust code!");
  });

  it("an_unknown_skill_is_a_danger_chip", async () => {
    await form({ text: TEXT.replace("[test-driven-development]", "[ghost]"), builtin: false }, { builtin: false, overridden: true });
    expect(button("ghost ×").closest(".skill-chip")).toHaveAttribute("data-tone", "danger");
  });

  it("an_unmapped_error_shows_on_the_bottom_line", async () => {
    const message = "specialists/rust.md: permission denied";
    const savePersonaDefinition = vi.fn().mockRejectedValue(new FleetError(message));
    await form(ownCopy, { builtin: false, overridden: true }, { savePersonaDefinition });
    await userEvent.type(field("Summary"), "!");
    await userEvent.click(button("Save"));
    const shown = await screen.findByText(message);
    expect(shown.parentElement).toHaveClass("specialist-form");
    expect(field("Summary")).toHaveValue("Rust code!");
  });

  it("preview_renders_the_body", async () => {
    await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.click(button("Preview"));
    expect(screen.getByRole("heading", { name: "You are a Rust specialist" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Instructions" })).toBeNull();
    await userEvent.click(button("Edit"));
    expect(field("Instructions")).toBeInTheDocument();
  });

  it("revert_restores_fields_body_and_nickname", async () => {
    await form(ownCopy, { builtin: false, overridden: true, nickname: "Ferris" });
    await userEvent.type(field("Nickname"), "x");
    await userEvent.type(field("Summary"), "!");
    await userEvent.type(field("Instructions"), "more");
    await userEvent.click(button("Revert"));
    expect(field("Nickname")).toHaveValue("Ferris");
    expect(field("Summary")).toHaveValue("Rust code");
    expect(field("Instructions")).toHaveValue("# You are a Rust specialist\n\nWrite tests first.\n");
    expect(button("Save")).toBeDisabled();
  });

  it("a_saved_nickname_tells_the_apps_persona_list", async () => {
    const onNick = vi.fn();
    await form({}, {}, {}, { onNick });
    await userEvent.type(field("Nickname"), "Ferris");
    await userEvent.click(button("Save"));
    await waitFor(() => expect(onNick).toHaveBeenCalledTimes(1));
  });

  it("a_failed_nickname_save_shows_under_nickname_and_keeps_the_draft", async () => {
    const onNick = vi.fn();
    await form({}, {}, { setNickname: vi.fn().mockRejectedValue(new FleetError("too long")) }, { onNick });
    await userEvent.type(field("Nickname"), "Ferris");
    await userEvent.click(button("Save"));
    const shown = await screen.findByText(/too long/);
    expect(shown).toHaveClass("error-line");
    expect(screen.getByText("Nickname").closest("div")).toContainElement(shown);
    expect(field("Nickname")).toHaveValue("Ferris");
    expect(onNick).not.toHaveBeenCalled();
    await userEvent.type(field("Nickname"), "!");
    expect(screen.queryByText(/too long/)).toBeNull();
  });

  it("switching_persona_drops_the_nickname_draft", async () => {
    const c = await form();
    await userEvent.type(field("Nickname"), "Ferris");
    c.view.rerender(c.ui({ persona: "b" }));
    await waitFor(() => expect(c.personaDefinition).toHaveBeenCalledWith("b", "specialist", "rust"));
    expect(field("Nickname")).toHaveValue("");
  });

  it("switching_specialist_drops_the_nickname_draft", async () => {
    const c = await form();
    await userEvent.type(field("Nickname"), "Ferris");
    c.view.rerender(c.ui({ name: "go" }));
    await waitFor(() => expect(c.personaDefinition).toHaveBeenCalledWith("a", "specialist", "go"));
    expect(field("Nickname")).toHaveValue("");
  });

  it("reset_to_built_in_asks_then_resets", async () => {
    const resetPersonaDefinition = vi.fn().mockResolvedValue(undefined);
    await form(ownCopy, { builtin: true, overridden: true }, { resetPersonaDefinition });
    await userEvent.click(button("Reset to built-in"));
    expect(resetPersonaDefinition).not.toHaveBeenCalled();
    await userEvent.click(button("Reset"));
    expect(resetPersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust");
  });

  it("one_save_writes_the_definition_and_the_nickname", async () => {
    const client = await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.type(field("Summary"), "!");
    await userEvent.type(field("Nickname"), "Ferris");
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust", TEXT.replace("Rust code", "Rust code!"));
    expect(client.setNickname).toHaveBeenCalledWith("a", "rust", "Ferris");
  });

  it("a_refused_definition_does_not_stop_the_nickname", async () => {
    const message = "specialists/rust.md: permission denied";
    const savePersonaDefinition = vi.fn().mockRejectedValue(new FleetError(message));
    const client = await form(ownCopy, { builtin: false, overridden: true }, { savePersonaDefinition });
    await userEvent.type(field("Summary"), "!");
    await userEvent.type(field("Nickname"), "Ferris");
    await userEvent.click(button("Save"));
    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(client.setNickname).toHaveBeenCalledWith("a", "rust", "Ferris");
    expect(field("Summary")).toHaveValue("Rust code!");
  });

  it("a_cleared_field_keeps_its_place_when_retyped", async () => {
    const client = await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.click(field("Summary"));
    await userEvent.keyboard("{End}{Backspace>9/}");
    expect(field("Summary")).toHaveValue("");
    await userEvent.type(field("Summary"), "Go code");
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust", TEXT.replace("Rust code", "Go code"));
  });

  it("a_field_left_empty_is_dropped_from_the_saved_file", async () => {
    const client = await form(ownCopy, { builtin: false, overridden: true });
    await userEvent.click(field("Model"));
    await userEvent.keyboard("{End}{Backspace>6/}");
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust", TEXT.replace("model: sonnet\n", ""));
  });

  describe("moving between roles", () => {
    const movable = (over: Record<string, unknown> = {}) =>
      form(ownCopy, { builtin: false, overridden: true }, { movePersonaDefinition: vi.fn().mockResolvedValue(undefined), ...over }, { roleFixed: false });

    it("role_change_moves_and_reopens", async () => {
      const client = await movable();
      await userEvent.click(screen.getByRole("combobox", { name: "Role" }));
      await userEvent.click(screen.getByRole("option", { name: "Orchestrator" }));
      expect(client.movePersonaDefinition).toHaveBeenCalledWith("a", "specialist", "rust", "orchestrator");
      await waitFor(() => expect(client.personaDefinition).toHaveBeenCalledWith("a", "orchestrator", "rust"));
      await waitFor(() => expect(client.personaDefinitions).toHaveBeenCalledTimes(2));
      expect(await screen.findByRole("combobox", { name: "Role" })).toHaveTextContent("Orchestrator");
    });

    it("a_refused_move_keeps_the_form_and_shows_why", async () => {
      const message = "orchestrators/rust.md already exists";
      const client = await movable({ movePersonaDefinition: vi.fn().mockRejectedValue(new FleetError(message)) });
      await userEvent.click(screen.getByRole("combobox", { name: "Role" }));
      await userEvent.click(screen.getByRole("option", { name: "Orchestrator" }));
      const shown = await screen.findByText(message);
      expect(shown).toHaveClass("error-line");
      expect(screen.getByText("Role").closest("div")).toContainElement(shown);
      expect(screen.getByRole("combobox", { name: "Role" })).toHaveTextContent("Specialist");
      expect(field("Summary")).toHaveValue("Rust code");
      expect(client.personaDefinition).toHaveBeenCalledTimes(1);
    });

    it("a_dirty_form_cannot_move", async () => {
      await movable();
      await userEvent.type(field("Summary"), "!");
      expect(screen.getByRole("combobox", { name: "Role" })).toBeDisabled();
    });

    it("an_unmodified_builtin_keeps_its_role_read_only", async () => {
      await form({}, {}, {}, { roleFixed: false });
      expect(screen.queryByRole("combobox", { name: "Role" })).toBeNull();
      expect(screen.getByText("Specialist")).toBeInTheDocument();
    });
  });

  it("form_css_uses_tokens_only", () => {
    const css = readFileSync("src/management/specialistForm.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/gradient/);
    expect(css).not.toMatch(/border-radius/);
    expect(css).toMatch(/fui-wipe/);
  });
});


describe("New definition", () => {
  const list = [
    info(),
    info({ name: "go", builtin: false }),
    info({ kind: "orchestrator", name: "lead", builtin: false }),
    info({ name: "rust2", builtin: true, overridden: true }),
  ];

  let latest: ReturnType<typeof useDefinitions>;

  function Tab(p: { client: ReturnType<typeof fakeClient>; kind?: DefKind; roleFixed?: boolean }) {
    const defs = useDefinitions(p.client, "a", vi.fn());
    latest = defs;
    useEffect(() => {
      defs.loadList();
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    // With the role free, the tab shows whichever role is open, as the merged tab list will.
    const kind = p.roleFixed === false ? (defs.st.selected?.kind ?? p.kind ?? "specialist") : (p.kind ?? "specialist");
    return <Definitions kind={kind} defs={defs} roleFixed={p.roleFixed} />;
  }

  async function dialog(kind: DefKind = "specialist", roleFixed = true) {
    const client = fakeClient({
      personaDefinitions: vi.fn().mockResolvedValue(list),
      personaDefinition: vi.fn(async (_p: string, k: DefKind, name: string) => ({ kind: k, name, text: TEXT, builtin: true })),
    });
    render(<Tab client={client} kind={kind} roleFixed={roleFixed} />);
    await userEvent.click(await screen.findByRole("button", { name: kind === "specialist" ? "+ New specialist" : "+ New orchestrator" }));
    return client;
  }

  it("new_refuses_a_builtin_name", async () => {
    await dialog();
    await userEvent.type(field("Name"), "rust");
    expect(screen.getByText("That's a built-in: open it and Override.")).toHaveClass("error-line");
    expect(button("OK")).toBeDisabled();
  });

  it("new_refuses_a_name_the_persona_has", async () => {
    await dialog();
    await userEvent.type(field("Name"), "lead");
    expect(screen.getByText("This persona already has lead.")).toBeInTheDocument();
    expect(button("OK")).toBeDisabled();
    await userEvent.clear(field("Name"));
    await userEvent.type(field("Name"), "rust2");
    expect(screen.getByText("This persona already has rust2.")).toBeInTheDocument();
  });

  it("new_refuses_a_name_the_daemon_would", async () => {
    await dialog();
    await userEvent.type(field("Name"), "-bad name");
    expect(screen.getByText(/letters, digits/)).toHaveClass("error-line");
    expect(button("OK")).toBeDisabled();
  });

  it("new_opens_an_unsaved_draft_under_the_chosen_role", async () => {
    const client = await dialog("specialist", false);
    await userEvent.type(field("Name"), "scout");
    await userEvent.click(screen.getByRole("combobox", { name: "Role" }));
    await userEvent.click(screen.getByRole("option", { name: "Orchestrator" }));
    await userEvent.click(button("OK"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(client.savePersonaDefinition).not.toHaveBeenCalled();
    expect(field("Name")).toHaveValue("scout");
    expect(field("Name")).toHaveAttribute("readonly");
    expect(field("Instructions")).toHaveValue("# scout\n");
    expect(screen.queryByRole("button", { name: "Override" })).toBeNull();
    await userEvent.type(field("Summary"), "Scouts");
    await userEvent.click(button("Save"));
    expect(client.savePersonaDefinition).toHaveBeenCalledWith("a", "orchestrator", "scout", "---\nname: scout\nsummary: Scouts\n---\n# scout\n");
    await waitFor(() => expect(client.personaDefinitions).toHaveBeenCalledTimes(2));
  });

  it("new_with_a_fixed_role_presets_the_tab_kind", async () => {
    await dialog("orchestrator");
    expect(screen.getByRole("combobox", { name: "Role" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Role" })).toHaveTextContent("Orchestrator");
  });

  it("selecting_another_row_drops_an_unsaved_draft", async () => {
    const client = await dialog();
    await userEvent.type(field("Name"), "scout");
    await userEvent.click(button("OK"));
    expect(field("Name")).toHaveValue("scout");
    await userEvent.click(screen.getByRole("button", { name: /^go / }));
    await waitFor(() => expect(client.personaDefinition).toHaveBeenCalledWith("a", "specialist", "go"));
    await waitFor(() => expect(field("Name")).toHaveValue("go"));
    expect(Object.keys(latest.st.drafts).some((k) => k.endsWith("/scout"))).toBe(false);
  });
});
