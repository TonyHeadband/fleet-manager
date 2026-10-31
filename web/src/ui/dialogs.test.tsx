import { act, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { motion as timing } from "../generated/layout";
import { END_SLACK } from "../shell/entry";
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

  describe("closing", () => {
    const ghost = () => document.querySelector<HTMLElement>("body > .dialog-backdrop[data-closing]");
    const open = () =>
      render(
        <Modal label="New product" onClose={() => {}}>
          <h2>New product</h2>
        </Modal>,
      );
    afterEach(() => {
      delete document.documentElement.dataset.animations;
      document.querySelectorAll("body > .dialog-backdrop[data-closing]").forEach((g) => g.remove());
      vi.useRealTimers();
    });

    it("a_closed_dialog_plays_its_opening_backwards_then_goes", () => {
      open().unmount();
      // However it was closed (Cancel, Save, Escape, a press outside), a still copy closes in its place.
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(ghost()).toHaveAttribute("inert");
      expect(ghost()).toHaveAttribute("aria-hidden", "true");
      expect(ghost()).toHaveTextContent("New product");
      fireEvent.animationEnd(ghost()!.querySelector(".dialog")!, { animationName: "fui-dialog-close" });
      expect(ghost()).toBeNull();
    });

    it("a_lost_end_still_clears_the_copy", () => {
      vi.useFakeTimers();
      open().unmount();
      act(() => vi.advanceTimersByTime(timing.dialogClose + END_SLACK + 1));
      expect(ghost()).toBeNull();
    });

    it("with_motion_off_it_goes_at_once", () => {
      document.documentElement.dataset.animations = "off";
      open().unmount();
      expect(ghost()).toBeNull();
    });

    it("the_close_is_the_open_reversed_and_faster", () => {
      expect(reduced).toMatch(
        /\.dialog-backdrop\[data-closing\] \{ animation: fui-undim var\(--motion-dialog-close\) var\(--ease-pull\) both; \}/,
      );
      expect(reduced).toMatch(
        /\.dialog-backdrop\[data-closing\] \.dialog \{ animation: fui-dialog-close var\(--motion-dialog-close\) var\(--ease-pull\) both; \}/,
      );
      expect(reduced).toMatch(
        /\.dialog-backdrop\[data-closing\] \.dialog::after \{ animation: fui-dialog-line-close var\(--motion-dialog-close\) var\(--ease-pull\) both; \}/,
      );
      expect(motion).toMatch(
        /@keyframes fui-dialog-close \{ from \{ clip-path: inset\(0\); \} 60% \{ clip-path: inset\(calc\(50% - 1px\) 0\); \} to \{ clip-path: inset\(calc\(50% - 1px\) 50%\); \} \}/,
      );
      // The line shows only once the card has closed onto it, never over text.
      expect(motion).toMatch(/@keyframes fui-dialog-line-close \{ from, 58% \{ opacity: 0; \} 60% \{ opacity: 1; \} \}/);
      expect(timing.dialogClose).toBeLessThan(timing.dialog);
    });
  });
});
