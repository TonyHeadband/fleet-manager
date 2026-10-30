import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Badge } from "./Badge";
import { Field } from "./Field";
import { Button } from "./Button";
import { Tabs } from "./Tabs";
import { SmallIconButton } from "./SmallIconButton";
import { Form } from "./Form";
import { FleetError } from "../api/client";
import { Column } from "../components/Column";
import type { FieldDef } from "../api/generated/FieldDef";
import { agent, form, schema, workspace } from "../test/fixtures";

describe("Button", () => {
  it("is a non-submitting button unless told otherwise", () => {
    render(
      <form onSubmit={(e) => e.preventDefault()}>
        <Button>Go</Button>
        <Button type="submit">Send</Button>
      </form>,
    );
    expect(screen.getByRole("button", { name: "Go" })).toHaveAttribute("type", "button");
    expect(screen.getByRole("button", { name: "Send" })).toHaveAttribute("type", "submit");
  });

  it("carries its variant and pressed state as attributes the stylesheet keys on", () => {
    render(
      <>
        <Button variant="danger">Confirm</Button>
        <Button variant="quiet-danger">Kill</Button>
        <Button pressed>On</Button>
        <Button>Plain</Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "Confirm" })).toHaveAttribute("data-variant", "danger");
    expect(screen.getByRole("button", { name: "Kill" })).toHaveAttribute("data-variant", "quiet-danger");
    expect(screen.getByRole("button", { name: "On" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Plain" })).not.toHaveAttribute("data-variant");
    expect(screen.getByRole("button", { name: "Plain" })).not.toHaveAttribute("aria-pressed");
  });

  it("passes clicks and disabled through", async () => {
    const onClick = vi.fn();
    const { rerender } = render(<Button onClick={onClick}>Go</Button>);
    await userEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
    rerender(<Button disabled onClick={onClick}>Go</Button>);
    expect(screen.getByRole("button")).toBeDisabled();
  });
});

describe("Badge", () => {
  it("shows its text with the label as a tooltip", () => {
    render(<Badge label="Repos" field="repos">core</Badge>);
    const b = screen.getByTitle("Repos");
    expect(b).toHaveTextContent("core");
    expect(b).toHaveClass("badge");
    expect(b).toHaveAttribute("data-field", "repos");
  });
});

describe("Tabs", () => {
  const tabs = [{ id: "a", label: "Alpha" }, { id: "b", label: "Beta" }] as const;

  it("marks the selected tab and reports clicks", async () => {
    const onSelect = vi.fn();
    render(<Tabs tabs={tabs} selected="a" onSelect={onSelect} />);
    expect(screen.getByRole("tab", { name: "Alpha" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Beta" })).toHaveAttribute("aria-selected", "false");
    await userEvent.click(screen.getByRole("tab", { name: "Beta" }));
    expect(onSelect).toHaveBeenCalledWith("b");
  });

  it("renders extra controls after the tabs without making them tabs", () => {
    render(
      <Tabs tabs={tabs} selected="b" onSelect={vi.fn()}>
        <Button>Refresh</Button>
      </Tabs>,
    );
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(within(screen.getByRole("tablist")).getByRole("button", { name: "Refresh" })).toBeInTheDocument();
  });
});

describe("Column", () => {
  const working = schema.columns.find((c) => c.id === "working")!;
  const props = { schema, now: 1300, onRun: vi.fn(), onOpen: vi.fn() };

  it("Column_renders_its_title_count_and_cards_oldest_first", () => {
    const agents = [agent({ id: "n", goal: "newer", status_since: 900 }), agent({ id: "o", goal: "older", status_since: 100 })];
    render(<Column def={working} agents={agents} {...props} />);
    const region = screen.getByRole("region", { name: "Working" });
    expect(region).toHaveAttribute("data-tone", "active");
    // Painted "02", read "2", as Lite's mono_label_as.
    expect(within(region).getByRole("heading", { name: "Working 2" })).toBeInTheDocument();
    expect(within(region).getAllByRole("article")[0]).toHaveTextContent("older");
  });

  it("shows the schema's empty hint when nothing is in it", () => {
    render(<Column def={working} agents={[]} {...props} />);
    expect(screen.getByText(working.empty_hint)).toBeInTheDocument();
    expect(screen.queryByRole("article")).not.toBeInTheDocument();
  });

  it("does not reorder the caller's list", () => {
    const agents = [agent({ id: "n", status_since: 900 }), agent({ id: "o", status_since: 100, workspaces: [workspace("x")] })];
    const before = agents.map((a) => a.id);
    render(<Column def={working} agents={agents} {...props} />);
    expect(agents.map((a) => a.id)).toEqual(before);
  });
});

describe("Field", () => {
  const def = (over: Partial<FieldDef>): FieldDef => ({ key: "goal", label: "Task", kind: "text", show_when: { type: "always" }, value_labels: {}, value_tones: {}, ...over });

  it("renders each kind the schema can ask for", () => {
    const a = agent({ status_since: 1000, goal: "do it", activity: "running Bash" });
    const { container } = render(
      <>
        <Field def={def({ key: "goal", kind: "text" })} agent={a} now={1300} />
        <Field def={def({ key: "branch", label: "Branch", kind: "mono" })} agent={a} now={1300} />
        <Field def={def({ key: "status_since", label: "In status", kind: "duration" })} agent={a} now={1300} />
        <Field def={def({ key: "activity", label: "Activity", kind: "badge" })} agent={a} now={1300} />
      </>,
    );
    expect(screen.getByText("do it")).toHaveAttribute("data-field", "goal");
    expect(container.querySelector("code.mono")).toHaveTextContent("fleet/fix-the-tests");
    expect(screen.getByTitle("In status")).toHaveTextContent("5m");
    expect(screen.getByTitle("Activity")).toHaveClass("badge");
  });

  it("renders nothing for an empty, false or missing value", () => {
    const a = agent({ activity: "", paused: false, blocked_reason: null });
    const { container } = render(
      <>
        <Field def={def({ key: "activity", kind: "text" })} agent={a} now={1} />
        <Field def={def({ key: "paused", kind: "badge" })} agent={a} now={1} />
        <Field def={def({ key: "blocked_reason", kind: "badge" })} agent={a} now={1} />
        <Field def={def({ key: "no_such_field", kind: "text" })} agent={a} now={1} />
      </>,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows a true boolean as its label and a list as one element per entry", () => {
    const a = agent({ paused: true, workspaces: [workspace("core"), workspace("portal")] });
    render(
      <>
        <Field def={def({ key: "paused", label: "Paused", kind: "badge" })} agent={a} now={1} />
        <Field def={def({ key: "repos", label: "Repos", kind: "badge" })} agent={a} now={1} />
      </>,
    );
    expect(screen.getByTitle("Paused")).toHaveTextContent("Paused");
    expect(screen.getAllByTitle("Repos").map((b) => b.textContent)).toEqual(["core", "portal"]);
  });
});

describe("Form", () => {
  const create = form("release_create");

  it("draws the inputs and blocks Create until the version has text", async () => {
    render(<Form form={create} today="2026-10-29" onSubmit={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "New release" })).toBeInTheDocument();
    for (const label of ["Version", "Start", "Target"]) expect(screen.getByLabelText(label)).toBeInTheDocument();
    const go = screen.getByRole("button", { name: "Create" });
    expect(go).toBeDisabled();
    await userEvent.type(screen.getByLabelText("Version"), "1.0");
    expect(go).toBeEnabled();
  });

  it("puts a refusal under the input it names, or under the form", async () => {
    const onSubmit = vi
      .fn()
      .mockRejectedValueOnce(new FleetError('"soon" is not a date; use YYYY-MM-DD', "target"))
      .mockRejectedValueOnce(new FleetError("1.0 is already taken"));
    render(<Form form={create} today="2026-10-29" onSubmit={onSubmit} />);
    await userEvent.type(screen.getByLabelText("Version"), "1.0");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    const msg = await screen.findByText('"soon" is not a date; use YYYY-MM-DD');
    expect(msg.closest("[data-key]")).toHaveAttribute("data-key", "target");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    const top = await screen.findByText("1.0 is already taken");
    expect(top.closest("[data-key]")).toBeNull();
  });
});

describe("Form with several actions", () => {
  const breakdown = form("breakdown_decision");

  it("draws each action, each enabled by what it needs", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<Form form={breakdown} today="2026-10-29" onSubmit={onSubmit} />);
    const changes = screen.getByRole("button", { name: "Request changes" });
    expect(changes).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    await userEvent.type(screen.getByLabelText("Or ask for changes"), "smaller");
    expect(changes).toBeEnabled();
    await userEvent.click(changes);
    expect(onSubmit).toHaveBeenCalledWith(breakdown.more[0], { comment: "smaller" });
  });

  it("puts a refusal for the comment under it, and one for another key at form level", async () => {
    const onSubmit = vi
      .fn()
      .mockRejectedValueOnce(new FleetError("comment is required", "comment"))
      .mockRejectedValueOnce(new FleetError("no such task", "owner"));
    render(<Form form={breakdown} today="2026-10-29" onSubmit={onSubmit} />);
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    const msg = await screen.findByText("comment is required");
    expect(msg.closest("[data-key]")).toHaveAttribute("data-key", "comment");
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect((await screen.findByText("no such task")).closest("[data-key]")).toBeNull();
  });

  it("offers a choice from its source", () => {
    render(<Form form={form("repo_request")} today="2026-10-29" sources={{ product_repos: ["core", "portal"] }} onSubmit={vi.fn()} />);
    const select = screen.getByLabelText("Grant access to");
    expect([...select.querySelectorAll("option")].map((o) => o.textContent)).toEqual(["", "core", "portal"]);
    expect(screen.getByRole("button", { name: "Grant" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refuse" })).toBeEnabled();
  });
});

describe("SmallIconButton", () => {
  it("small_icon_buttons_are_named_by_their_action", () => {
    render(<SmallIconButton icon="pause" label="Pause" onClick={() => {}} />);
    const b = screen.getByRole("button", { name: "Pause" });
    expect(b).toHaveAttribute("title", "Pause");
    expect(b.querySelector("svg")).not.toBeNull();
  });
  it("a_danger_icon_is_marked", () => {
    render(<SmallIconButton icon="stop" label="Click again to stop" danger onClick={() => {}} />);
    expect(screen.getByRole("button", { name: "Click again to stop" })).toHaveAttribute("data-danger");
  });
});
