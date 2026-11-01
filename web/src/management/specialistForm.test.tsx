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

function Host(p: { client: ReturnType<typeof fakeClient>; kind?: DefKind; info: DefInfo; skills?: string[] }) {
  const defs = useDefinitions(p.client, "a", vi.fn());
  const kind = (p.kind ?? "specialist") as "specialist" | "orchestrator";
  useEffect(() => {
    defs.loadList();
    defs.open(kind, "rust");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const listed = defs.st.list?.find((d) => d.name === "rust") ?? p.info;
  if (!defs.st.files[defs.key(kind, "rust")]) return <p>Loading…</p>;
  return <SpecialistForm persona="a" kind={kind} name="rust" info={listed} defs={defs} skills={p.skills ?? SKILLS} roleFixed />;
}

async function form(file: Partial<DefFile> = {}, over: Partial<DefInfo> = {}, clientOver: Record<string, unknown> = {}) {
  const client = fakeClient({
    personaDefinition: vi.fn().mockResolvedValue({ kind: "specialist", name: "rust", text: TEXT, builtin: true, ...file }),
    personaDefinitions: vi.fn().mockResolvedValue([info(over)]),
    ...clientOver,
  });
  render(<Host client={client} info={info(over)} />);
  await screen.findByText("Summary");
  return client;
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

  it("form_css_uses_tokens_only", () => {
    const css = readFileSync("src/management/specialistForm.css", "utf8");
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/gradient/);
    expect(css).not.toMatch(/border-radius/);
    expect(css).toMatch(/fui-wipe/);
  });
});

