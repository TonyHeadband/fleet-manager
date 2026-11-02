import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { panel } from "../generated/layout";
import { Folding } from "./Folding";

/**
 * jsdom lays nothing out: each text node's lines end at the given bottoms, in px from the box's top. With no line
 * height to read, the fold's budget is `panel.foldMax`.
 */
function linesEndAt(bottoms: number[]) {
  Range.prototype.getClientRects = function () {
    return bottoms.map((b) => ({ bottom: b })) as unknown as DOMRectList;
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  delete (Range.prototype as { getClientRects?: unknown }).getClientRects;
});

describe("Folding", () => {
  it("a_one_line_message_with_a_paragraph_margin_offers_no_read_more", () => {
    // The box is taller than its one line (the paragraph's margin): that is no reason to fold.
    linesEndAt([18]);
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(30);
    render(
      <Folding>
        <p>one line</p>
      </Folding>,
    );
    expect(screen.queryByRole("button", { name: "Read more" })).toBeNull();
  });

  it("a_message_with_a_line_past_the_eighth_folds_at_the_last_whole_line", () => {
    const last = panel.foldMax - 4;
    linesEndAt([18, last, panel.foldMax + 16]);
    render(
      <Folding>
        <p>nine lines</p>
      </Folding>,
    );
    expect(screen.getByRole("button", { name: "Read more" })).toBeInTheDocument();
    const text = document.querySelector<HTMLElement>(".chat-fold-text")!;
    expect(text).toHaveAttribute("data-folded");
    expect(text.style.getPropertyValue("--fold-limit")).toBe(`${last}px`);
  });
});
