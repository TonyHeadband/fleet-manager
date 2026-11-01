import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ToastStack, useToasts, type ToastItem } from "./Toast";

afterEach(() => vi.useRealTimers());

const item = (over: Partial<ToastItem> = {}): Omit<ToastItem, "id"> => ({ title: "fix", text: "wants you", tone: "accent", ...over });

describe("Toast", () => {
  it("at_most_three_and_each_leaves_after_eight_seconds", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useToasts());
    act(() => {
      for (const n of [1, 2, 3, 4]) result.current.push(item({ title: `t${n}` }));
    });
    expect(result.current.toasts.map((t) => t.title)).toEqual(["t2", "t3", "t4"]);
    act(() => vi.advanceTimersByTime(7_999));
    expect(result.current.toasts).toHaveLength(3);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current.toasts).toHaveLength(0);
  });

  it("a_toast_shows_its_tone_and_leaves_on_click", async () => {
    const onDismiss = vi.fn();
    render(<ToastStack toasts={[{ id: 7, title: "fix", text: "failed", tone: "danger", href: "#supervisor?c=s1" }]} onDismiss={onDismiss} />);
    const toast = screen.getByRole("button", { name: /fix/ });
    expect(toast.querySelector(".toast-mark")).toHaveAttribute("data-tone", "danger");
    await userEvent.click(toast);
    expect(onDismiss).toHaveBeenCalledWith(7);
    expect(window.location.hash).toBe("#supervisor?c=s1");
  });

  it("the_status_region_is_there_before_the_first_toast_so_it_is_announced", () => {
    const { rerender } = render(<ToastStack toasts={[]} onDismiss={vi.fn()} />);
    const region = screen.getByRole("status");
    expect(region).toBeEmptyDOMElement();
    rerender(<ToastStack toasts={[{ id: 1, ...item() }]} onDismiss={vi.fn()} />);
    expect(screen.getByRole("status")).toBe(region);
    expect(region).toHaveTextContent("wants you");
  });

  it("styles_use_the_chat_tones_and_motion_stays_gated", () => {
    const css = readFileSync("src/ui/ui.css", "utf8");
    expect(css).toMatch(/\.toast-mark\[data-tone="accent"\] \{[^}]*background: var\(--fill\);/);
    expect(css).toMatch(/\.toast-mark\[data-tone="danger"\] \{[^}]*background: var\(--danger\);/);
    const motion = readFileSync("src/shell/motion.css", "utf8");
    expect(motion).toMatch(/prefers-reduced-motion: no-preference\) \{[^@]*\.toast/);
    expect(motion).toMatch(/\.toast \{\s*animation: fui-wipe/);
  });
});
