import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Footer } from "./Footer";
import { Messages, type Notice } from "./messages";

const msg = (notice: Notice, text: string, time = 0, agentId?: string) => ({ notice, text, time, agentId });

describe("Footer", () => {
  it("the_footer_shows_one_line_and_its_history", async () => {
    const m = new Messages(false);
    m.push(msg("error", "first line\nsecond line", 3600 + 5 * 60), 0);
    m.push(msg("info", "saved", 7200), 0);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} />);
    const line = screen.getByText("first line second line");
    expect(line).toHaveAttribute("title", "first line\nsecond line");
    expect(screen.getByText("1/2")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    const rows = screen.getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual(["02:00saved", "01:05first line second line"]);
    expect(screen.getByRole("button", { name: "Open log →" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Open log →" })).toHaveAttribute("title", "not on the web yet");
  });

  it("the_footer_counter_shows_the_message_on_screen", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("error", "E2"), 0);
    m.tick(8.1);
    render(<Footer messages={m} onClick={() => {}} onSelect={() => {}} />);
    expect(screen.getByText("E2")).toBeInTheDocument();
    expect(screen.getByText("2/2")).toBeInTheDocument();
  });

  it("the_footer_line_is_clickable_only_when_a_click_does_something", async () => {
    const m = new Messages(false);
    m.push(msg("info", "saved"), 0);
    const onClick = vi.fn();
    const { rerender } = render(<Footer messages={m} onClick={onClick} onSelect={() => {}} />);
    await userEvent.click(screen.getByRole("button", { name: "saved" }));
    expect(onClick).toHaveBeenCalled();
    m.tick(4.1);
    rerender(<Footer messages={m} onClick={onClick} onSelect={() => {}} />);
    expect(screen.queryByRole("button", { name: "saved" })).not.toBeInTheDocument();
    expect(screen.getByText("saved")).toBeInTheDocument();
  });

  it("a_message_about_an_agent_selects_its_card", async () => {
    const m = new Messages(false);
    m.push(msg("error", "a1 crashed", 0, "a1"), 0);
    const onSelect = vi.fn();
    render(<Footer messages={m} onClick={() => {}} onSelect={onSelect} />);
    await userEvent.click(screen.getByRole("button", { name: "a1 crashed" }));
    expect(onSelect).toHaveBeenCalledWith("a1");
  });
});
