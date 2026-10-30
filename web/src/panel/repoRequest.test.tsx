import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { form } from "../test/fixtures";
import { RepoRequestBox } from "./RepoRequestBox";

// Ported from crates/fleet-gui/tests/repo_request.rs; the names are Lite's.
function show(over: Partial<Parameters<typeof RepoRequestBox>[0]> = {}) {
  const props = {
    request: { repo: "portal", why: "the login page lives there" },
    productRepos: ["core", "portal"],
    form: form("repo_request"),
    draft: { target: "portal", reason: "" },
    onDraft: vi.fn(),
    blocked: null,
    focus: false,
    onGrant: vi.fn(),
    onRefuse: vi.fn(),
    ...over,
  };
  render(<RepoRequestBox {...props} />);
  return props;
}

describe("RepoRequestBox", () => {
  it("shows_what_the_agent_wants_and_why", async () => {
    const { onGrant, onDraft } = show();
    expect(screen.getByText("Wants portal")).toBeInTheDocument();
    expect(screen.getByText("the login page lives there")).toBeInTheDocument();
    expect(screen.getByText("Grant access to")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "portal" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "core" })).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(screen.getByRole("button", { name: "core" }));
    expect(onDraft).toHaveBeenCalledWith({ target: "core", reason: "" });
    await userEvent.click(screen.getByRole("button", { name: "Grant" }));
    expect(onGrant).toHaveBeenCalledWith("portal");
  });

  it("the_request_box_follows_the_daemons_description", () => {
    const described = form("repo_request");
    described.submit.label = "Allow";
    show({ form: described });
    expect(screen.getByRole("button", { name: "Allow" })).toBeInTheDocument();
  });

  it("a_repo_outside_the_product_cannot_be_granted", () => {
    show({ request: { repo: "billing", why: "" }, draft: { target: "billing", reason: "" } });
    expect(screen.getByText("billing is not a repo of this product: add it in Products first, or refuse")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Grant" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refuse" })).toBeEnabled();
  });

  it("refuse_sends_the_reason", async () => {
    const { onRefuse } = show({ draft: { target: "portal", reason: "  not this one  " } });
    expect(screen.getByLabelText("Reason (optional)")).toHaveValue("  not this one  ");
    await userEvent.click(screen.getByRole("button", { name: "Refuse" }));
    expect(onRefuse).toHaveBeenCalledWith("not this one");
  });

  it("ctrl_enter_in_the_reason_field_does_not_grant", async () => {
    const { onGrant, onRefuse } = show();
    await userEvent.type(screen.getByLabelText("Reason (optional)"), "x{Enter}{Control>}{Enter}{/Control}");
    expect(onGrant).not.toHaveBeenCalled();
    expect(onRefuse).not.toHaveBeenCalled();
  });

  it("a_blocked_box_says_why", () => {
    show({ blocked: "A decision is being sent" });
    expect(screen.getByRole("button", { name: "Grant" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Grant" })).toHaveAttribute("title", "A decision is being sent");
    expect(screen.getByRole("button", { name: "Refuse" })).toBeDisabled();
  });
});
