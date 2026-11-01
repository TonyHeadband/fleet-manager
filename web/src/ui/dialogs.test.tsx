import { act, fireEvent, render, screen } from "@testing-library/react";
import { StrictMode } from "react";
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

  it("a_dialog_is_drawn_left_to_right_in_one_wipe", () => {
    // The HUD draws a panel from left to right, as a view's sections enter (fui-wipe).
    expect(reduced).toMatch(/\.dialog \{ animation: fui-wipe var\(--motion-dialog\) var\(--ease-push\) both; \}/);
    expect(motion).not.toMatch(/\.dialog::after|fui-dialog-line|@keyframes fui-dialog \{/);
    expect(reduced).toMatch(/\.dialog-backdrop \{ animation: fui-dim var\(--motion-dialog\) var\(--ease-push\) both; \}/);
    expect(motion).toMatch(/@keyframes fui-wipe \{ from \{ clip-path: inset\(0 100% 0 0\); \} to \{ clip-path: inset\(0\); \} \}/);
    expect(readFileSync("src/generated/theme.css", "utf8")).toContain("--motion-dialog: 380ms;");
  });

  it("new_task_spaces_its_selects_as_it_did_the_browsers", () => {
    // The app's select (Select.tsx) takes the place and the spacing the native one had.
    const css = readFileSync("src/newTask/newTask.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.new-task \.select \{ width: calc\(100% - var\(--new-task-control-inset\)\); margin-bottom: var\(--new-task-field-gap\); \}/);
    expect(css).toMatch(/\.new-task \.new-task-start \.select \{ width: calc\(var\(--new-task-width\) - var\(--new-task-start-inset\)\); margin-bottom: 0; \}/);
    expect(css).not.toMatch(/(?<![.\w-])select \{/);
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
    /** The close is decided a microtask after the unmount, once the dialog is known to be gone. */
    const close = async () => {
      open().unmount();
      await act(async () => {});
    };
    afterEach(() => {
      delete document.documentElement.dataset.animations;
      document.querySelectorAll("body > .dialog-backdrop[data-closing]").forEach((g) => g.remove());
      vi.useRealTimers();
    });

    it("a_closed_dialog_plays_its_opening_backwards_then_goes", async () => {
      await close();
      // However it was closed (Cancel, Save, Escape, a press outside), a still copy closes in its place.
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(ghost()).toHaveAttribute("inert");
      expect(ghost()).toHaveAttribute("aria-hidden", "true");
      expect(ghost()).toHaveTextContent("New product");
      fireEvent.animationEnd(ghost()!.querySelector(".dialog")!, { animationName: "fui-dialog-close" });
      expect(ghost()).toBeNull();
    });

    it("a_lost_end_still_clears_the_copy", async () => {
      vi.useFakeTimers();
      await close();
      act(() => vi.advanceTimersByTime(timing.dialogClose + END_SLACK + 1));
      expect(ghost()).toBeNull();
    });

    it("with_motion_off_it_goes_at_once", async () => {
      document.documentElement.dataset.animations = "off";
      await close();
      expect(ghost()).toBeNull();
    });

    it("opening_under_strict_mode_plays_no_close", async () => {
      // StrictMode mounts, unmounts and remounts in development: that unmount is not a close.
      render(
        <StrictMode>
          <Modal label="New product" onClose={() => {}}>
            <h2>New product</h2>
          </Modal>
        </StrictMode>,
      );
      await act(async () => {});
      expect(ghost()).toBeNull();
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    it("the_close_is_the_open_reversed_and_faster", () => {
      expect(reduced).toMatch(
        /\.dialog-backdrop\[data-closing\] \{ animation: fui-undim var\(--motion-dialog-close\) var\(--ease-pull\) both; \}/,
      );
      expect(reduced).toMatch(
        /\.dialog-backdrop\[data-closing\] \.dialog \{ animation: fui-dialog-close var\(--motion-dialog-close\) var\(--ease-pull\) both; \}/,
      );
      // The wipe backwards: the card retracts to its left edge.
      expect(motion).toMatch(/@keyframes fui-dialog-close \{ from \{ clip-path: inset\(0\); \} to \{ clip-path: inset\(0 100% 0 0\); \} \}/);
      expect(timing.dialogClose).toBeLessThan(timing.dialog);
    });
  });
});
