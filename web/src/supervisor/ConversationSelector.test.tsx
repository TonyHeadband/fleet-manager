import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Conversation } from "../api/generated/Conversation";
import { ConversationSelector } from "./ConversationSelector";

const DAY = 86_400;
const NOON = Math.floor(new Date(2026, 9, 31, 12).getTime() / 1000);
const conv = (id: string, title: string | null, last_used_at: number, tasks: Conversation["tasks"] = []): Conversation => ({
  id,
  persona: "me",
  title,
  created_at: last_used_at,
  last_used_at,
  live: false,
  tasks,
});

describe("ConversationSelector", () => {
  it("selector_rows_show_title_date_and_task_marks", async () => {
    vi.useFakeTimers({ toFake: ["Date"], now: NOON * 1000 });
    try {
      const rows = [
        conv("c1", "Brainstorm", NOON - 60, [
          { id: "t1", title: "a", status: "working" },
          { id: "t2", title: "b", status: "review" },
          { id: "t3", title: "c", status: "needs_you" },
        ]),
        conv("c2", null, NOON - DAY),
        conv("c3", "Old", new Date(2026, 9, 30 - 2, 12).getTime() / 1000),
      ];
      const onPick = vi.fn();
      render(<ConversationSelector conversations={rows} current="c1" onPick={onPick} onNew={() => {}} />);
      await userEvent.click(screen.getByRole("button", { name: "Conversation" }));
      const items = within(document.querySelector<HTMLElement>(".menu")!).getAllByRole("button");
      expect(items[0]).toHaveTextContent("Brainstorm");
      expect(items[0]).toHaveTextContent("today");
      expect([...items[0]!.querySelectorAll(".dot")].map((d) => d.getAttribute("data-tone"))).toEqual(["active", "done", "attention"]);
      expect(items[1]).toHaveTextContent("Untitled");
      expect(items[1]).toHaveTextContent("yesterday");
      expect(items[2]).toHaveTextContent("Oct 28");
      expect(items[2]!.querySelectorAll(".dot")).toHaveLength(0);
      expect(items[0]).toHaveAttribute("aria-pressed", "true");
      await userEvent.click(items[2]!);
      expect(onPick).toHaveBeenCalledWith("c3");
    } finally {
      vi.useRealTimers();
    }
  });

  it("the_caption_sits_outside_and_new_calls_onNew", async () => {
    const onNew = vi.fn();
    render(<ConversationSelector conversations={[]} current={null} onPick={() => {}} onNew={onNew} />);
    expect(screen.getByRole("button", { name: "Conversation" }).closest(".filter-menu")).toHaveAttribute("data-caption", "side");
    await userEvent.click(screen.getByRole("button", { name: "+ New" }));
    expect(onNew).toHaveBeenCalledTimes(1);
  });
});
