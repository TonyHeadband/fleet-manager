import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync, readdirSync } from "node:fs";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { ListDetail } from "./ListDetail";
import { ListRow } from "./ListRow";
import { PairRowsEditor, RowsEditor } from "./RowsEditor";
import { SaveBar } from "./SaveBar";
import { Section } from "./Section";

// The shapes the ported views share (egui's list_row, Panel::left + CentralPanel, section captions, save_bar and
// the rule rows), so the polish after the port changes one place.

const css = readFileSync("src/ui/ui.css", "utf8");

describe("shared shapes", () => {
  it("list_detail_scrolls_each_side_at_its_width", () => {
    render(<ListDetail width="management" listLabel="Specialists" detailLabel="Definition" list={<p>rows</p>} detail={<p>file</p>} />);
    expect(screen.getByText("rows").closest(".list-detail-list")).toHaveAttribute("aria-label", "Specialists");
    expect(screen.getByRole("region", { name: "Definition" })).toHaveClass("list-detail-detail");
    expect(css).toMatch(/\.list-detail-list \{[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.list-detail\[data-width="task"\] \.list-detail-list \{[^}]*width: var\(--layout-task-list\)/);
    expect(css).toMatch(/\.list-detail\[data-width="management"\] \.list-detail-list \{[^}]*width: var\(--layout-management-list\)/);
    expect(css).toMatch(/\.list-detail-detail \{[^}]*flex: 1/);
    expect(css).toMatch(/\.list-detail-detail \{[^}]*overflow-y: auto/);
  });

  it("list_row_is_a_pressed_button_named_by_its_item", async () => {
    const onSelect = vi.fn();
    render(
      <ListRow name="alpha" selected={false} onSelect={onSelect}>
        <span>alpha</span>
        <span>details</span>
      </ListRow>,
    );
    const row = screen.getByRole("button", { name: "alpha" });
    expect(row).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(row);
    expect(onSelect).toHaveBeenCalled();
    expect(css).toMatch(/\.list-row \{[^}]*padding: var\(--layout-list-row-y\) var\(--layout-list-row-x\)/);
  });

  it("section_has_a_mono_caption", () => {
    render(<Section title="Goal">text</Section>);
    expect(screen.getByText("Goal")).toHaveClass("mono-label");
  });

  it("save_bar_enables_save_when_dirty_or_forced_and_revert_when_dirty", async () => {
    const onSave = vi.fn();
    const onRevert = vi.fn();
    const { rerender } = render(<SaveBar dirty={false} busy={false} onSave={onSave} onRevert={onRevert} />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Revert" })).toBeDisabled();
    expect(screen.getByText("Applies to agents started after saving.")).toBeInTheDocument();
    rerender(<SaveBar dirty={false} force busy={false} onSave={onSave} onRevert={onRevert} />);
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Revert" })).toBeDisabled();
    rerender(<SaveBar dirty busy={false} onSave={onSave} onRevert={onRevert} />);
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await userEvent.click(screen.getByRole("button", { name: "Revert" }));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onRevert).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute("data-variant", "primary");
    rerender(<SaveBar dirty busy onSave={onSave} onRevert={onRevert} />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("rows_editor_adds_and_removes_named_rows", async () => {
    function Harness() {
      const [rows, setRows] = useState(["Bash(ls:*)"]);
      return <RowsEditor rows={rows} noun="allow rule" hint="Bash(git status:*)" onChange={setRows} />;
    }
    render(<Harness />);
    expect(screen.getByRole("textbox", { name: "allow rule 1" })).toHaveValue("Bash(ls:*)");
    await userEvent.click(screen.getByRole("button", { name: "Add allow rule" }));
    expect(screen.getByRole("textbox", { name: "allow rule 2" })).toHaveAttribute("placeholder", "Bash(git status:*)");
    await userEvent.click(screen.getByRole("button", { name: "Remove allow rule 1" }));
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(screen.getByRole("textbox", { name: "allow rule 1" })).toHaveValue("");
  });

  it("pair_rows_editor_names_each_field", async () => {
    function Harness() {
      const [rows, setRows] = useState<[string, string][]>([["A", "1"]]);
      return <PairRowsEditor rows={rows} onChange={setRows} />;
    }
    render(<Harness />);
    expect(screen.getByRole("textbox", { name: "Variable name 1" })).toHaveValue("A");
    expect(screen.getByRole("textbox", { name: "Variable value 1" })).toHaveValue("1");
    await userEvent.click(screen.getByRole("button", { name: "Add variable" }));
    expect(screen.getByRole("textbox", { name: "Variable name 2" })).toHaveAttribute("placeholder", "NAME");
    await userEvent.click(screen.getByRole("button", { name: "Remove variable 1" }));
    expect(screen.getAllByRole("textbox")).toHaveLength(2);
  });

  it("every_css_variable_is_defined", () => {
    // A renamed token leaves var(--old) silently empty; every variable a stylesheet reads must be declared somewhere.
    const files = (readdirSync("src", { recursive: true }) as string[])
      .filter((f) => f.endsWith(".css"))
      .map((f) => readFileSync(`src/${f}`, "utf8"));
    const all = files.join("\n");
    const declared = new Set([...all.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
    const read = new Set([...all.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]));
    expect([...read].filter((v) => !declared.has(v))).toEqual([]);
  });

  it("the_shared_css_uses_tokens_only", () => {
    expect(css.match(/(?<![\w.])(?!1px\b)\d+(\.\d+)?px\b/g)).toBeNull();
  });
});
