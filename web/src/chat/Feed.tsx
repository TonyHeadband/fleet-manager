import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

/** How long the scrollbar stays drawn after the last scroll. */
const HIDE_AFTER_MS = 700;
/** Under this many px from an end counts as being there. */
const AT_EDGE = 2;

type Cues = { above: boolean; below: boolean };

/**
 * A chat's scrolling feed. Its scrollbar shows only while it moves, and a three-line arrow above and below says more
 * is there, each going once that end is reached. The arrows sit in bands of their own, never over the text.
 */
export function Feed({ feedRef, label, onScroll, children }: { feedRef: RefObject<HTMLDivElement | null>; label: string; onScroll?: (el: HTMLDivElement) => void; children: ReactNode }) {
  const [cues, setCues] = useState<Cues>({ above: false, below: false });
  const [scrolling, setScrolling] = useState(false);
  const hide = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const measure = () => {
    const el = feedRef.current;
    if (!el) return;
    const above = el.scrollTop > AT_EDGE;
    const below = el.scrollHeight - el.scrollTop - el.clientHeight > AT_EDGE;
    setCues((c) => (c.above === above && c.below === below ? c : { above, below }));
  };

  // Content changes the height without a scroll (a new post, a fold opening), so the cues follow the DOM too.
  useLayoutEffect(() => {
    const el = feedRef.current;
    if (!el) return;
    measure();
    const mo = new MutationObserver(measure);
    mo.observe(el, { childList: true, subtree: true, characterData: true });
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    ro?.observe(el);
    return () => {
      mo.disconnect();
      ro?.disconnect();
      clearTimeout(hide.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- observers set up once; measure reads the ref
  }, []);

  return (
    <div className="chat">
      <Cue edge="top" shown={cues.above} />
      <div
        ref={feedRef}
        role="log"
        aria-label={label}
        className="chat-feed"
        data-scrolling={scrolling || undefined}
        onScroll={(e) => {
          measure();
          setScrolling(true);
          clearTimeout(hide.current);
          hide.current = setTimeout(() => setScrolling(false), HIDE_AFTER_MS);
          onScroll?.(e.currentTarget);
        }}
      >
        {children}
      </div>
      <Cue edge="bottom" shown={cues.below} />
    </div>
  );
}

function Cue({ edge, shown }: { edge: "top" | "bottom"; shown: boolean }) {
  return (
    <div className="chat-cue" data-edge={edge} data-shown={shown || undefined} aria-hidden="true">
      <span />
      <span />
      <span />
    </div>
  );
}
