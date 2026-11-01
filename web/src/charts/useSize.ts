import { useLayoutEffect, useRef, useState, type RefObject } from "react";

/** Size starts at 0×0 (hidden tab, before layout): blocks draw no marks until a real size arrives. */
export function useSize<T extends Element>(): [RefObject<T>, { width: number; height: number }] {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[entries.length - 1]?.contentRect;
      if (!r) return;
      setSize((s) => (s.width === r.width && s.height === r.height ? s : { width: r.width, height: r.height }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref as RefObject<T>, size];
}
