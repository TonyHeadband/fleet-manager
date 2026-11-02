import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { panel } from "../generated/layout";
import { useSize } from "../charts/useSize";
import { Button } from "../ui/Button";

/** A message folds past this many of its own lines. */
const FOLD_LINES = 8;

/**
 * Where a message folds, in px from its top: eight of the element's own lines (`panel.foldMax` is that for the
 * chat's body text, and stands in when the line height cannot be read), then snapped up to the bottom of the last
 * whole line within it, so paragraph margins never leave the cut through a line.
 */
function foldLimit(el: HTMLElement): number {
  const lineHeight = parseFloat(window.getComputedStyle(el.firstElementChild ?? el).lineHeight);
  const budget = Number.isFinite(lineHeight) ? lineHeight * FOLD_LINES : panel.foldMax;
  const top = el.getBoundingClientRect().top;
  let snapped = 0;
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    range.selectNodeContents(node);
    if (typeof range.getClientRects !== "function") break;
    for (const r of Array.from(range.getClientRects())) {
      const bottom = r.bottom - top;
      if (bottom <= budget + 0.5 && bottom > snapped) snapped = bottom;
    }
  }
  return snapped > 0 ? snapped : budget;
}

/**
 * A message's text, folded past eight whole lines of measured height: cut clean on a line boundary, with a Read more /
 * Show less button under it. A short message shows no button.
 */
export function Folding({ children }: { children: ReactNode }) {
  const [text, size] = useSize<HTMLDivElement>();
  const box = useRef<HTMLDivElement>(null);
  const [limit, setLimit] = useState<number>(panel.foldMax);
  const [tall, setTall] = useState(false);
  const [open, setOpen] = useState(false);
  // Measured at the full height (a folded box is clipped, its scrollHeight is not), again when the text changes and
  // when the panel is resized (the Diff and Fullscreen widen it, so the text wraps onto fewer lines).
  useLayoutEffect(() => {
    const el = text.current;
    if (!el) return;
    const at = foldLimit(el);
    setLimit(at);
    setTall(el.scrollHeight > at);
  }, [children, size.width, text]);
  const toggle = () => {
    setOpen(!open);
    // Folding back shortens the feed: keep the message in view rather than let the scroll land elsewhere.
    if (open) box.current?.scrollIntoView?.({ block: "nearest" });
  };
  const folded = tall && !open;
  return (
    <div ref={box} className="chat-fold">
      <div ref={text} className="chat-fold-text" data-folded={folded ? "" : undefined} style={{ "--fold-limit": `${limit}px` } as CSSProperties}>
        {children}
      </div>
      {tall && (
        <Button className="chat-fold-toggle" aria-expanded={open} onClick={toggle}>
          {open ? "Show less" : "Read more"}
        </Button>
      )}
    </div>
  );
}
