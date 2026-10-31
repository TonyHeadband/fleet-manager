import { fireEvent, render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Page } from "./Page";

/** jsdom lays nothing out: give the page the sizes a browser would. */
function sized(el: HTMLElement, { height, view, top }: { height: number; view: number; top: number }) {
  Object.defineProperty(el, "scrollHeight", { configurable: true, value: height });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: view });
  el.scrollTop = top;
  fireEvent.scroll(el);
}

describe("Page", () => {
  it("a_page_is_a_tabbed_page_with_its_own_classes", () => {
    const { container } = render(<Page className="settings-page">x</Page>);
    expect(container.firstChild).toHaveClass("scroll-frame");
    expect(container.firstChild!.firstChild).toHaveClass("tabbed-page", "scroller", "settings-page");
  });

  it("a_scroller_marks_the_edges_with_more_beyond_them", () => {
    const { container } = render(<Page>x</Page>);
    const page = container.firstChild!.firstChild as HTMLElement;
    sized(page, { height: 200, view: 200, top: 0 });
    expect(page).not.toHaveAttribute("data-more");
    sized(page, { height: 600, view: 200, top: 0 });
    expect(page).toHaveAttribute("data-more", "below");
    sized(page, { height: 600, view: 200, top: 150 });
    expect(page).toHaveAttribute("data-more", "above below");
    sized(page, { height: 600, view: 200, top: 400 });
    expect(page).toHaveAttribute("data-more", "above");
  });
});

describe("scroll affordance styles", () => {
  const css = readFileSync("src/styles.css", "utf8");

  it("the_scrollbar_is_drawn_so_it_stays_visible", () => {
    // WebKitGTK's overlay scrollbars hide until hovered; a styled one is always drawn, in the theme.
    expect(css).toMatch(/::-webkit-scrollbar \{[^}]*width: var\(--layout-scrollbar\)/);
    expect(css).toMatch(/::-webkit-scrollbar-thumb \{[^}]*background: var\(--fill\)/);
  });

  it("nothing_fades_a_scrollers_edges", () => {
    // A gradient fade is not the app's look; what marks more content is drawn over the frame instead.
    expect(readFileSync("src/ui/ui.css", "utf8")).not.toMatch(/mask|--scroll-tint/);
  });
});
