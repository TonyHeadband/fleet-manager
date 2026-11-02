import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { Composer } from "./Composer";

const box = (open: boolean, draft = false) => (
  <Composer label="Post" initiallyOpen={open} draft={draft}>
    <textarea placeholder="Post…" />
  </Composer>
);

describe("Composer", () => {
  it("folds_and_opens_on_its_header", async () => {
    render(box(true));
    const head = screen.getByRole("button", { name: "Post" });
    expect(head).toHaveAttribute("aria-expanded", "true");
    // Open from the first draw, it leaves the cursor where it was.
    expect(screen.getByPlaceholderText("Post…")).not.toHaveFocus();
    await userEvent.click(head);
    expect(screen.queryByPlaceholderText("Post…")).toBeNull();
    expect(head).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(head);
    expect(screen.getByPlaceholderText("Post…")).toHaveFocus();
  });

  it("a_folded_header_says_a_draft_waits", () => {
    const { rerender } = render(box(false));
    expect(screen.queryByText("· draft")).toBeNull();
    rerender(box(false, true));
    expect(screen.getByText("· draft")).toBeInTheDocument();
  });
});
