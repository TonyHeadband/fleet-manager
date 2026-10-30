import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { FleetError } from "../api/client";
import type { Persona } from "../api/generated/Persona";
import type { Native } from "../native/native";
import { FakeEventSource, factory, fakeClient, fakeNative, persona } from "../test/fixtures";
import { PersonaSetup } from "./PersonaSetup";

// Ported from crates/fleet-gui/tests/persona_setup.rs and the wizard's persona steps in shell.rs.
const list = (...personas: Persona[]) => ({
  personas,
  claude_command: ["claude"],
  gh_command: "gh",
  tasks_root: "/home/me/fleet-tasks",
  claude_path: null,
  gh_path: null,
});

function draft(over: Partial<Persona> = {}): Persona {
  return {
    ...persona("p1", false),
    name: "Work",
    host: "github.com",
    commit_name: "Me",
    commit_email: "me@example.invalid",
    github_auth: "gh",
    claude_dir: "/p/claude",
    gh_dir: "/p/gh",
    claude: null,
    github: null,
    ...over,
  } as Persona;
}

const ok = (account: string) => ({ ok: true, account, detail: null, tool: null });

function setup(
  over: {
    client?: Parameters<typeof fakeClient>[0];
    native?: Native | null;
    draft?: Persona | null;
    steps?: { folders: boolean; requirements: boolean };
  } = {},
) {
  const client = fakeClient({
    personas: vi.fn().mockResolvedValue(list()),
    createPersona: vi.fn().mockResolvedValue(draft()),
    checkPersona: vi.fn().mockResolvedValue(draft()),
    deletePersona: vi.fn().mockResolvedValue(undefined),
    ...over.client,
  });
  const native = over.native === undefined ? fakeNative() : over.native;
  const onFinish = vi.fn();
  render(
    <PersonaSetup
      client={client}
      native={native}
      local
      steps={over.steps ?? { folders: false, requirements: false }}
      stillMissing={[]}
      draft={over.draft ?? null}
      onFinish={onFinish}
      onChanged={() => {}}
    />,
  );
  return { client, native, onFinish };
}

async function fillIdentity() {
  await userEvent.type(await screen.findByLabelText("Name"), "Work");
  await userEvent.type(screen.getByLabelText("Commit name"), "Me");
  await userEvent.type(screen.getByLabelText("Commit email"), "me@example.invalid");
}

describe("Persona setup, as egui's", () => {
  beforeEach(() => {
    FakeEventSource.reset();
    localStorage.clear();
  });
  afterEach(() => vi.useRealTimers());

  it("no_ready_persona_shows_wizard_and_blocks_new_task", async () => {
    render(<App client={fakeClient({ personas: vi.fn().mockResolvedValue(list()) })} eventSource={factory} nowSeconds={() => 1300} />);
    expect(await screen.findByRole("heading", { name: "Step 1 of 2 · Your first persona" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New task" })).toHaveAttribute("title", "Set up a persona first");
  });

  it("next_creates_the_persona_with_github_com_by_default", async () => {
    const { client } = setup();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    await fillIdentity();
    expect(screen.getByLabelText("GitHub host")).toHaveAttribute("placeholder", "github.com");
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(client.createPersona).toHaveBeenCalledWith({
      name: "Work",
      host: "github.com",
      commit_name: "Me",
      commit_email: "me@example.invalid",
      github_auth: "gh",
    });
    expect(await screen.findByRole("heading", { name: "Step 2 of 2 · Log in" })).toBeInTheDocument();
    expect(screen.getByText("Each opens a terminal running the real CLI. Claude starts in /home/me/fleet-tasks.")).toBeInTheDocument();
    expect(screen.getByText("GitHub · github.com")).toBeInTheDocument();
  });

  it("create_error_shows_under_form", async () => {
    setup({ client: { createPersona: vi.fn().mockRejectedValue(new FleetError("A persona named Work exists")) } });
    await fillIdentity();
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("A persona named Work exists")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Step 1 of 2 · Your first persona" })).toBeInTheDocument();
  });

  it("the_login_step_checks_the_persona", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { client } = setup({ draft: draft() });
    await waitFor(() => expect(client.checkPersona).toHaveBeenCalledWith("p1"));
    await act(() => vi.advanceTimersByTimeAsync(3000));
    expect(client.checkPersona).toHaveBeenCalledTimes(2);
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(client.checkPersona).toHaveBeenCalledTimes(3);
  });

  it("finish_disabled_until_both_logins_ok", async () => {
    const checkPersona = vi
      .fn()
      .mockResolvedValueOnce(draft({ claude: ok("me@claude") }))
      .mockResolvedValue(draft({ ready: true, claude: ok("me@claude"), github: ok("me") }));
    const { native, onFinish } = setup({ draft: draft(), client: { checkPersona } });
    expect(await screen.findByText("me@claude")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Finish" })).toBeEnabled());
    await userEvent.click(screen.getByRole("checkbox", { name: "Add an app menu shortcut" }));
    await userEvent.click(screen.getByRole("button", { name: "Finish" }));
    expect(native!.addMenuShortcut).toHaveBeenCalled();
    expect(onFinish).toHaveBeenCalled();
  });

  it("pat_field_names_the_permissions_it_needs", async () => {
    const pat = draft({ github_auth: "pat" });
    setup({ draft: pat, client: { checkPersona: vi.fn().mockResolvedValue(pat) } });
    expect(
      await screen.findByText(
        "Fine-grained token: give it the repos this persona works on, with Contents and Pull requests: Read and write (Metadata: Read is added for you). A classic token needs the repo and read:org scopes.",
      ),
    ).toBeInTheDocument();
  });

  it("pat_field_is_masked_and_cleared_after_submit", async () => {
    const pat = draft({ github_auth: "pat" });
    const { native } = setup({ draft: pat, client: { checkPersona: vi.fn().mockResolvedValue(pat) } });
    const field = await screen.findByLabelText("Personal access token");
    expect(field).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "Save token" })).toBeDisabled();
    await userEvent.type(field, "ghp_secret");
    await userEvent.click(screen.getByRole("button", { name: "Save token" }));
    expect(native!.savePat).toHaveBeenCalledWith("p1", "ghp_secret");
    expect(field).toHaveValue("");
  });

  it("cancel_on_login_deletes_draft", async () => {
    const { client } = setup();
    await fillIdentity();
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(client.deletePersona).toHaveBeenCalledWith("p1");
    expect(await screen.findByRole("heading", { name: "Step 1 of 2 · Your first persona" })).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toHaveValue("Work");
  });

  it("no_terminal_shows_the_script_to_copy", async () => {
    const { native } = setup({
      draft: draft(),
      native: fakeNative({ runLogin: vi.fn().mockResolvedValue({ script: "cd /t && claude" }) }),
    });
    const [claude] = await screen.findAllByRole("button", { name: "Log in…" });
    await userEvent.click(claude!);
    expect(native!.runLogin).toHaveBeenCalledWith("p1", "claude");
    expect(await screen.findByText("No terminal found. Run this in one:")).toBeInTheDocument();
    expect(screen.getByText("cd /t && claude")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(native!.copyText).toHaveBeenCalledWith("cd /t && claude");
  });

  it("the_browser_shows_every_login_as_a_script_to_copy", async () => {
    setup({ draft: draft(), native: null, client: { personas: vi.fn().mockResolvedValue(list(draft())) } });
    const [claude] = await screen.findAllByRole("button", { name: "Log in…" });
    await userEvent.click(claude!);
    expect(await screen.findByText("No terminal found. Run this in one:")).toBeInTheDocument();
    expect(screen.getByText(/^cd \/home\/me\/fleet-tasks && env .*CLAUDE_CONFIG_DIR=\/p\/claude claude$/)).toBeInTheDocument();
  });

  it("a_draft_left_behind_resumes_at_login", async () => {
    render(
      <App
        client={fakeClient({ personas: vi.fn().mockResolvedValue(list(draft())), checkPersona: vi.fn().mockResolvedValue(draft()) })}
        eventSource={factory}
        nowSeconds={() => 1300}
      />,
    );
    expect(await screen.findByRole("heading", { name: "Step 2 of 2 · Log in" })).toBeInTheDocument();
  });

  it("cancel_is_hidden_once_the_persona_is_ready", async () => {
    setup({
      draft: draft(),
      client: { checkPersona: vi.fn().mockResolvedValue(draft({ ready: true, claude: ok("a"), github: ok("b") })) },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "Finish" })).toBeEnabled());
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("a_check_landing_after_cancel_does_not_make_the_draft_active", async () => {
    let land!: (p: Persona) => void;
    const checkPersona = vi.fn(() => new Promise<Persona>((r) => (land = r)));
    setup({ draft: draft(), client: { checkPersona } });
    await waitFor(() => expect(checkPersona).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await act(async () => land(draft({ ready: true, claude: ok("a"), github: ok("b") })));
    expect(screen.getByRole("heading", { name: "Step 1 of 2 · Your first persona" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Finish" })).toBeNull();
  });

  it("a_check_error_for_another_persona_is_not_shown", async () => {
    let fail!: (e: Error) => void;
    const checkPersona = vi
      .fn()
      .mockImplementationOnce(() => new Promise<Persona>((_, r) => (fail = r)))
      .mockResolvedValue(draft({ id: "p2" }));
    setup({ draft: draft(), client: { checkPersona, createPersona: vi.fn().mockResolvedValue(draft({ id: "p2" })) } });
    await waitFor(() => expect(checkPersona).toHaveBeenCalledWith("p1"));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    await screen.findByRole("heading", { name: "Step 2 of 2 · Log in" });
    await act(async () => fail(new Error("p1 is gone")));
    expect(screen.queryByText(/Could not check the logins/)).toBeNull();
  });

  it("an_upgrade_without_the_folders_step_counts_three_steps", async () => {
    setup({ steps: { folders: false, requirements: true } });
    expect(await screen.findByRole("heading", { name: "Step 2 of 3 · Your first persona" })).toBeInTheDocument();
  });
});
