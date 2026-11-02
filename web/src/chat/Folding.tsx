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
 *
 * Tall means a line of text ends past those eight. The box's own height is no measure: a paragraph's margin makes even
 * a one-line message taller than its text, and every message would offer Read more.
 */
function measure(el: HTMLElement): { limit: number; tall: boolean } {
  const lineHeight = parseFloat(window.getComputedStyle(el.firstElementChild ?? el).lineHeight);
  const budget = Number.isFinite(lineHeight) ? lineHeight * FOLD_LINES : panel.foldMax;
  const top = el.getBoundingClientRect().top;
  let snapped = 0;
  let lines = false;
  let past = false;
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    range.selectNodeContents(node);
    if (typeof range.getClientRects !== "function") break;
    for (const r of Array.from(range.getClientRects())) {
      lines = true;
      const bottom = r.bottom - top;
      if (bottom > budget + 0.5) past = true;
      else if (bottom > snapped) snapped = bottom;
    }
  }
  // Without line boxes to read (no layout), the height is all there is.
  return { limit: snapped > 0 ? snapped : budget, tall: lines ? past : el.scrollHeight > budget };
}

/**
 * A message's text, folded past eight whole lines of measured height: cut clean on a line boundary, with a Read more /
 * Show less button under it. A short message shows no button. `text` is the message's source text, what the children
 * render: the message is measured again only when it or the panel width changes.
 */
export function Folding({ text, children }: { text: string; children: ReactNode }) {
  const [textRef, size] = useSize<HTMLDivElement>();
  const box = useRef<HTMLDivElement>(null);
  const [limit, setLimit] = useState<number>(panel.foldMax);
  const [tall, setTall] = useState(false);
  const [open, setOpen] = useState(false);
  // Measured at the full height (a folded box is clipped, its scrollHeight is not), again when the message's text
  // changes and when the panel is resized (the Diff and Fullscreen widen it, so the text wraps onto fewer lines).
  // Not on every render: `children` is a new element each time the parent renders, and a long chat would measure
  // every message for each new line. `text` stands for the children, so it is the dependency.
  useLayoutEffect(() => {
    const el = textRef.current;
    if (!el) return;
    const m = measure(el);
    setLimit(m.limit);
    setTall(m.tall);
  }, [text, size.width, textRef]);
  const toggle = () => {
    setOpen(!open);
    // Folding back shortens the feed: keep the message in view rather than let the scroll land elsewhere.
    if (open) box.current?.scrollIntoView?.({ block: "nearest" });
  };
  const folded = tall && !open;
  return (
    <div ref={box} className="chat-fold">
      <div ref={textRef} className="chat-fold-text" data-folded={folded ? "" : undefined} style={{ "--fold-limit": `${limit}px` } as CSSProperties}>
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
