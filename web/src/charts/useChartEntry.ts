import { useEffect, useRef, useState } from "react";
import { motion } from "../generated/layout";
import { motionOn } from "../shell/entry";

/** The entry's last animation ends: the wipe, after the last staggered mark. */
const ENTRY_WINDOW = motion.wipe + motion.stagger * motion.staggerCap;

/**
 * True from the first render with data until the entry window ends, then false for good, even if the data empties
 * and returns. Always false with motion off. Staying true through the window keeps live updates from cutting the
 * marks' animation short.
 */
export function useChartEntry(hasData: boolean): boolean {
  const phase = useRef<"idle" | "run" | "over">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [, rerender] = useState(0);
  const enter = phase.current === "run" || (phase.current === "idle" && hasData && motionOn());
  useEffect(() => {
    if (hasData && phase.current === "idle") phase.current = motionOn() ? "run" : "over";
    if (phase.current === "run" && !timer.current) {
      timer.current = setTimeout(() => {
        phase.current = "over";
        rerender((n) => n + 1);
      }, ENTRY_WINDOW);
    }
  }, [hasData]);
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      timer.current = undefined;
    },
    [],
  );
  return enter;
}
