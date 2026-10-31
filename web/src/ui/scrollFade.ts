import { useCallback } from "react";

/**
 * A ref for a scroller that marks `data-more` with the edges that have more beyond them ("above", "below"), which
 * `.scroll-fade` fades out. It follows scrolling, the scroller's size and its content.
 */
export function useScrollFade<T extends HTMLElement>() {
  return useCallback((el: T | null) => {
    if (!el) return;
    const mark = () => {
      const more = [el.scrollTop > 1 && "above", el.scrollTop + el.clientHeight < el.scrollHeight - 1 && "below"].filter(Boolean);
      if (more.length) el.dataset.more = more.join(" ");
      else delete el.dataset.more;
    };
    mark();
    el.addEventListener("scroll", mark, { passive: true });
    const resized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(mark);
    resized?.observe(el);
    const changed = new MutationObserver(mark);
    changed.observe(el, { childList: true, subtree: true, characterData: true });
    return () => {
      el.removeEventListener("scroll", mark);
      resized?.disconnect();
      changed.disconnect();
    };
  }, []);
}
