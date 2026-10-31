import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Modal } from "./Modal";

// Every dialog (Modal) in the canvas's voice: a mono-caps title behind the accent marker bar, hairlines under the
// title and over the actions; it opens from an accent line drawn out from its centre.

const shell = readFileSync("src/shell/shell.css", "utf8").replace(/\s+/g, " ");
const motion = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
const reduced = motion.slice(motion.indexOf("@media (prefers-reduced-motion: no-preference)"));

describe("dialogs", () => {
  it("a_dialog_is_a_titled_instrument", () => {
    render(
      <Modal label="New product" onClose={() => {}}>
        <h2>New product</h2>
      </Modal>,
    );
    expect(screen.getByRole("dialog", { name: "New product" })).toHaveClass("dialog");
    expect(shell).toMatch(
      /\.dialog h2 \{[^}]*font-family: var\(--font-mono\);[^}]*letter-spacing: var\(--track-caps\);[^}]*text-transform: uppercase;[^}]*border-bottom: 1px solid var\(--border\);/,
    );
    expect(shell).toMatch(
      /\.dialog h2::before \{[^}]*width: var\(--layout-marker-w\);[^}]*height: var\(--layout-marker-h\);[^}]*background: var\(--fill\);/,
    );
    expect(shell).toMatch(/\.dialog > \.form-actions \{[^}]*border-top: 1px solid var\(--border\);/);
  });

  it("a_dialog_opens_from_an_accent_line", () => {
    expect(reduced).toMatch(/\.dialog \{ position: relative; animation: fui-dialog var\(--motion-dialog\) var\(--ease-push\) both; \}/);
    expect(reduced).toMatch(
      /\.dialog::after \{[^}]*top: 50%;[^}]*height: 1px;[^}]*background: var\(--fill\);[^}]*animation: fui-dialog-line var\(--motion-dialog\) var\(--ease-push\) both;/,
    );
    expect(reduced).toMatch(/\.dialog-backdrop \{ animation: fui-dim var\(--motion-dialog\) var\(--ease-push\) both; \}/);
    // First a 1px band grows out from the centre, then the card opens up and down from it.
    expect(motion).toMatch(
      /@keyframes fui-dialog \{ from \{ clip-path: inset\(calc\(50% - 1px\) 50%\); \} 40% \{ clip-path: inset\(calc\(50% - 1px\) 0\); \} to \{ clip-path: inset\(0\); \} \}/,
    );
    // The line is gone as soon as the card starts opening: it never lies over the dialog's text.
    expect(motion).toMatch(/@keyframes fui-dialog-line \{[^@]*40% \{ opacity: 1; \} 42%, to \{ opacity: 0; \} \}/);
    expect(readFileSync("src/generated/theme.css", "utf8")).toContain("--motion-dialog: 380ms;");
  });

  it("new_task_draws_its_hairlines_on_its_own_rows", () => {
    const css = readFileSync("src/newTask/newTask.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.new-task-head \{[^}]*padding-bottom: var\(--space-item\);[^}]*border-bottom: 1px solid var\(--border\);/);
    expect(css).toMatch(/\.new-task-head h2 \{ padding-bottom: 0; border-bottom: none; \}/);
    expect(css).toMatch(/\.new-task-actions \{[^}]*padding-top: var\(--space-item\);[^}]*border-top: 1px solid var\(--border\);/);
  });
});
