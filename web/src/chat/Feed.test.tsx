import { act, fireEvent, render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Feed } from "./Feed";

function Harness({ onScroll }: { onScroll?: (el: HTMLDivElement) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  return (
    <Feed feedRef={ref} label="Chat" onScroll={onScroll}>
      <p>a post</p>
    </Feed>
  );
}

/** jsdom lays nothing out: give the feed a 1000px tall content in a 200px window, scrolled to `top`. */
function scrollTo(feed: HTMLElement, top: number) {
  Object.defineProperty(feed, "scrollHeight", { configurable: true, value: 1000 });
  Object.defineProperty(feed, "clientHeight", { configurable: true, value: 200 });
  feed.scrollTop = top;
  fireEvent.scroll(feed);
}

const feedOf = (c: HTMLElement) => c.querySelector<HTMLElement>(".chat-feed")!;
const cue = (c: HTMLElement, edge: string) => c.querySelector(`.chat-cue[data-edge="${edge}"]`)!;

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** jsdom has no layout: a ResizeObserver the test fires by hand, as a message settling or the panel resizing would. */
function stubResize() {
  const fire: (() => void)[] = [];
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(cb: () => void) {
        fire.push(cb);
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  return () => act(() => fire.forEach((f) => f()));
}

/** The feed's content grows to `height` with no scroll, as a message rewrapping or folding does. */
function grow(feed: HTMLElement, height: number) {
  Object.defineProperty(feed, "scrollHeight", { configurable: true, value: height });
}

describe("Feed", () => {
  it("cues_show_more_above_and_below_and_go_at_each_end", () => {
    const { container } = render(<Harness />);
    const feed = feedOf(container);
    scrollTo(feed, 0);
    expect(cue(container, "top")).not.toHaveAttribute("data-shown");
    expect(cue(container, "bottom")).toHaveAttribute("data-shown");
    scrollTo(feed, 400);
    expect(cue(container, "top")).toHaveAttribute("data-shown");
    expect(cue(container, "bottom")).toHaveAttribute("data-shown");
    scrollTo(feed, 800);
    expect(cue(container, "top")).toHaveAttribute("data-shown");
    expect(cue(container, "bottom")).not.toHaveAttribute("data-shown");
  });

  it("each_cue_is_three_lines_kept_out_of_the_reading_order", () => {
    const { container } = render(<Harness />);
    for (const edge of ["top", "bottom"]) {
      expect(cue(container, edge).children).toHaveLength(3);
      expect(cue(container, edge)).toHaveAttribute("aria-hidden", "true");
    }
  });

  it("the_scrollbar_shows_while_scrolling_then_hides", () => {
    vi.useFakeTimers();
    const onScroll = vi.fn();
    const { container } = render(<Harness onScroll={onScroll} />);
    const feed = feedOf(container);
    expect(feed).not.toHaveAttribute("data-scrolling");
    scrollTo(feed, 100);
    expect(feed).toHaveAttribute("data-scrolling");
    expect(onScroll).toHaveBeenCalledWith(feed);
    act(() => vi.advanceTimersByTime(1000));
    expect(feed).not.toHaveAttribute("data-scrolling");
  });

  it("a_reader_at_the_end_stays_there_as_messages_settle", () => {
    const resized = stubResize();
    const { container } = render(<Harness />);
    const feed = feedOf(container);
    scrollTo(feed, 800);
    grow(feed, 1300);
    resized();
    expect(feed.scrollTop).toBe(1300);
  });

  it("a_reader_scrolled_up_or_clicking_in_the_feed_is_left_where_they_are", () => {
    const resized = stubResize();
    const { container } = render(<Harness />);
    const feed = feedOf(container);
    scrollTo(feed, 400);
    grow(feed, 1300);
    resized();
    expect(feed.scrollTop).toBe(400);
    // At the end, Read more on the last message grows it: its start stays where the reader clicked.
    scrollTo(feed, 1100);
    fireEvent.pointerDown(feed);
    grow(feed, 1600);
    resized();
    expect(feed.scrollTop).toBe(1100);
  });

  it("the_feed_has_no_scroll_anchoring", () => {
    // The browser's correction as messages settle would read as the reader leaving the end.
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-feed \{[^}]*overflow-anchor: none;/);
  });

  it("the_css_hides_the_idle_scrollbar_and_never_draws_cues_over_text", () => {
    const css = readFileSync("src/chat/chat.css", "utf8").replace(/\s+/g, " ");
    expect(css).toMatch(/\.chat-feed:not\(\[data-scrolling\]\)::-webkit-scrollbar-thumb, \.chat-feed:not\(\[data-scrolling\]\)::-webkit-scrollbar-track \{ background: transparent; \}/);
    const rule = css.match(/\.chat-cue \{([^}]*)\}/)![1]!;
    expect(rule).toContain("flex: none;");
    expect(rule).not.toMatch(/position: (absolute|fixed|sticky)/);
  });
});
