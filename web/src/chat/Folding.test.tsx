import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

type Observed = ((entries: { contentRect: { width: number; height: number } }[]) => void) | null;
let observed: Observed = null;

beforeEach(() => {
  observed = null;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(cb: Observed) {
        observed = cb;
      }
      observe() {}
      disconnect() {}
      unobserve() {}
    },
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (Range.prototype as { getClientRects?: unknown }).getClientRects;
});

describe("Folding", () => {
  it("a_one_line_message_with_a_paragraph_margin_offers_no_read_more", () => {
    // The box is taller than its one line (the paragraph's margin): that is no reason to fold.
    linesEndAt([18]);
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(30);
    render(
      <Folding text="one line">
        <p>one line</p>
      </Folding>,
    );
    expect(screen.queryByRole("button", { name: "Read more" })).toBeNull();
  });

  it("a_message_with_a_line_past_the_eighth_folds_at_the_last_whole_line", () => {
    const last = panel.foldMax - 4;
    linesEndAt([18, last, panel.foldMax + 16]);
    render(
      <Folding text="nine lines">
        <p>nine lines</p>
      </Folding>,
    );
    expect(screen.getByRole("button", { name: "Read more" })).toBeInTheDocument();
    const text = document.querySelector<HTMLElement>(".chat-fold-text")!;
    expect(text).toHaveAttribute("data-folded");
    expect(text.style.getPropertyValue("--fold-limit")).toBe(`${last}px`);
  });

  it("rerendering_with_the_same_text_measures_once", () => {
    linesEndAt([18]);
    const walk = vi.spyOn(document, "createTreeWalker");
    const { rerender } = render(
      <Folding text="same">
        <p>same</p>
      </Folding>,
    );
    const after = walk.mock.calls.length;
    expect(after).toBeGreaterThan(0);
    for (let i = 0; i < 3; i++)
      rerender(
        <Folding text="same">
          <p>same</p>
        </Folding>,
      );
    expect(walk.mock.calls.length).toBe(after);
  });

  it("new_text_is_measured_again_and_the_fold_follows_it", () => {
    linesEndAt([18]);
    const { rerender } = render(
      <Folding text="short">
        <p>short</p>
      </Folding>,
    );
    expect(screen.queryByRole("button", { name: "Read more" })).toBeNull();
    linesEndAt([18, panel.foldMax + 16]);
    rerender(
      <Folding text="tall">
        <p>tall</p>
      </Folding>,
    );
    expect(screen.getByRole("button", { name: "Read more" })).toBeInTheDocument();
  });

  it("a_height_change_with_the_same_width_and_text_is_measured_again_once", () => {
    // A font or weight setting rewraps the text: the box's height changes, its width and text do not.
    linesEndAt([18]);
    const walk = vi.spyOn(document, "createTreeWalker");
    render(
      <Folding text="same">
        <p>same</p>
      </Folding>,
    );
    const size = (width: number, height: number) => act(() => observed?.([{ contentRect: { width, height } }]));
    size(400, 100);
    const after = walk.mock.calls.length;
    size(400, 140);
    expect(walk.mock.calls.length).toBeGreaterThan(after);
    const again = walk.mock.calls.length;
    size(400, 140);
    expect(walk.mock.calls.length).toBe(again);
  });
});
