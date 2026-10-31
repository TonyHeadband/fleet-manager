import { useCallback, useRef } from "react";

export interface ScrollEdges {
  above: boolean;
  below: boolean;
  /** How far down it is scrolled, 0 to 100. */
  progress: number;
}

/**
 * A ref for a scroller that marks `data-more` with the edges that have more beyond them ("above", "below"), and tells
 * `onChange` where it is. It follows scrolling, the scroller's size and its content.
 */
export function useScrollEdges<T extends HTMLElement>(onChange?: (edges: ScrollEdges) => void) {
  const changed = useRef(onChange);
  changed.current = onChange;
  return useCallback((el: T | null) => {
    if (!el) return;
    const mark = () => {
      const range = el.scrollHeight - el.clientHeight;
      const above = el.scrollTop > 1;
      const below = el.scrollTop < range - 1;
      const more = [above && "above", below && "below"].filter(Boolean);
      if (more.length) el.dataset.more = more.join(" ");
      else delete el.dataset.more;
      changed.current?.({ above, below, progress: range > 0 ? Math.round((100 * el.scrollTop) / range) : 0 });
    };
    mark();
    el.addEventListener("scroll", mark, { passive: true });
    const resized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(mark);
    resized?.observe(el);
    const mutated = new MutationObserver(mark);
    mutated.observe(el, { childList: true, subtree: true, characterData: true });
    return () => {
      el.removeEventListener("scroll", mark);
      resized?.disconnect();
      mutated.disconnect();
    };
  }, []);
}
