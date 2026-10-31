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
import { Mark } from "./icons";
import { ByHand, CaptionField } from "../personas/logins";

// The shapes the ported views share (egui's list_row, Panel::left + CentralPanel, section captions, save_bar and
// the rule rows), so the polish after the port changes one place.

const css = readFileSync("src/ui/ui.css", "utf8");

describe("shared shapes", () => {
  it("list_detail_scrolls_each_side_at_its_width", () => {
    render(<ListDetail width="management" listLabel="Specialists" detailLabel="Definition" list={<p>rows</p>} detail={<p>file</p>} />);
    expect(screen.getByRole("region", { name: "Specialists" })).toHaveClass("list-detail-list");
    expect(screen.getByRole("region", { name: "Definition" })).toHaveClass("list-detail-detail");
    expect(css).toMatch(/\.list-detail-list \{[^}]*overflow-y: auto/);
    expect(css).toMatch(/\.list-detail\[data-width="task"\] \.list-detail-list \{[^}]*width: var\(--layout-task-list\)/);
    expect(css).toMatch(/\.list-detail\[data-width="management"\] \.list-detail-list \{[^}]*width: var\(--layout-management-list\)/);
    expect(css).toMatch(/\.list-detail-detail \{[^}]*flex: 1/);
    expect(css).toMatch(/\.list-detail-detail \{[^}]*overflow-y: auto/);
  });

  it("a_list_detail_is_linework_inside_its_view", () => {
    // The view's body is the one outline, as the board is; the list is divided from the detail by a hairline and,
    // like every section of the canvas, has no ground of its own.
    expect(css).not.toMatch(/\.list-detail-list \{[^}]*background/);
    expect(css).not.toMatch(/\.list-detail-detail \{[^}]*background/);
    expect(css).toMatch(/\.list-detail-list \{[^}]*border-right: 1px solid var\(--border\)/);
  });

  it("a_list_detail_pins_its_footer_under_the_list", () => {
    render(
      <ListDetail
        width="management"
        listLabel="Personas"
        detailLabel="Persona"
        list={<p>rows</p>}
        footer={<button>New</button>}
        detail={null}
      />,
    );
    const list = screen.getByText("rows").closest(".list-detail-list")!;
    const footer = screen.getByRole("button", { name: "New" }).closest(".list-detail-footer");
    expect(list).toContainElement(footer as HTMLElement);
    expect(screen.getByText("rows").compareDocumentPosition(footer!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(css).toMatch(/\.list-detail-footer \{[^}]*position: sticky/);
    // At the panel's bottom even when the list is short, as egui's Panel::bottom.
    expect(css).toMatch(/\.list-detail-footer \{[^}]*margin-top: auto/);
    expect(css).toMatch(/\.list-detail-list:has\(> \.list-detail-footer\) \{[^}]*flex-direction: column/);
  });

  it("the_ok_mark_names_itself", () => {
    render(<Mark name="ok" label="Both logins work" />);
    expect(screen.getByRole("img", { name: "Both logins work" })).toHaveAttribute("data-mark", "ok");
  });

  it("the_login_pieces_are_shared", () => {
    const { rerender } = render(<ByHand script="claude /login" native={null} />);
    expect(screen.getByText("No terminal found. Run this in one:")).toBeInTheDocument();
    expect(screen.getByText("claude /login")).toBeInTheDocument();
    rerender(<ByHand script={undefined} native={null} />);
    expect(screen.queryByText(/No terminal found/)).toBeNull();
    rerender(<CaptionField label="Commit name" value="Ann" onChange={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Commit name" })).toHaveValue("Ann");
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

  it("checkboxes_are_an_accent_outline", () => {
    // FUI: no fill, the accent border even when empty; a tick is an inner accent square.
    const css = readFileSync("src/styles.css", "utf8");
    expect(css).toMatch(
      /input\[type="checkbox"\] \{[^}]*appearance: none;[^}]*border: 1px solid var\(--fill\);[^}]*background: transparent;/,
    );
    expect(css).toMatch(/input\[type="checkbox"\]:checked \{[^}]*linear-gradient\(var\(--fill\), var\(--fill\)\)/);
  });

  it("status_marks_are_rectangles", () => {
    // No round dots: each is an upright bar of the canvas's size, wherever it stands; the changes mark a small square.
    const files = (readdirSync("src", { recursive: true }) as string[]).filter((f) => f.endsWith(".css"));
    for (const f of files) expect(readFileSync(`src/${f}`, "utf8"), f).not.toMatch(/border-radius: 50%|aspect-ratio|--dot:/);
    const shell = readFileSync("src/shell/shell.css", "utf8");
    expect(shell).toMatch(/\.dot, \.log-dot, \.log-dot-space \{[^}]*width: var\(--layout-bar-w\);[^}]*height: var\(--layout-bar-h\);/);
    expect(shell).toMatch(/\.changes-dot \{[^}]*width: var\(--layout-changes-dot\);[^}]*height: var\(--layout-changes-dot\);/);
  });

  it("controls_speak_in_mono_caps", () => {
    // The canvas's control voice: buttons, link buttons, selects and chips; icon buttons keep their own box.
    const css = readFileSync("src/styles.css", "utf8");
    expect(css).toMatch(
      /button, a\.button, select, \.chip \{[^}]*font-family: var\(--font-mono\);[^}]*font-size: var\(--size-meta\);[^}]*letter-spacing: var\(--track-caps\);[^}]*text-transform: uppercase;[^}]*padding: var\(--layout-control-y\) var\(--layout-control-x\);/,
    );
    expect(readFileSync("src/board/board.css", "utf8")).toMatch(/\.icon-button \{[^}]*padding: 0;/);
  });

  it("numbers_in_rails_and_columns_are_tabular", () => {
    // So a card's ticking time and a column's count keep their width.
    expect(readFileSync("src/styles.css", "utf8")).toMatch(/\.duration, \.mono \{[^}]*font-variant-numeric: tabular-nums;/);
    expect(readFileSync("src/board/board.css", "utf8")).toMatch(/\.count \{[^}]*font-variant-numeric: tabular-nums;/);
  });

  it("badges_are_mono_caps", () => {
    const css = readFileSync("src/styles.css", "utf8");
    expect(css).toMatch(
      /\.badge \{[^}]*font-family: var\(--font-mono\);[^}]*letter-spacing: var\(--track-value\);[^}]*text-transform: uppercase;[^}]*padding: 1px var\(--layout-badge-x\);/,
    );
  });

  it("a_picker_names_itself_in_the_theme_accent", () => {
    expect(readFileSync("src/board/board.css", "utf8")).toMatch(/\.filter-button \.mono-label \{ color: var\(--accent-text\); \}/);
  });

  it("corners_are_square", () => {
    // FUI keeps rigid angles: every control, card and dialog radius is 0, and no glyph rounds its rectangles.
    const theme = readFileSync("src/generated/theme.css", "utf8");
    for (const r of ["control", "card", "dialog"]) expect(theme).toContain(`--radius-${r}: 0px;`);
    expect(readFileSync("src/ui/icons.tsx", "utf8")).not.toMatch(/<rect[^>]*\brx=/);
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
