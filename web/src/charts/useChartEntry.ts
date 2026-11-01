import { useEffect, useRef } from "react";
import { motionOn } from "../shell/entry";

/** True only on the first render that has data, and only with motion on; later updates never replay it. */
export function useChartEntry(hasData: boolean): boolean {
  const played = useRef(false);
  const enter = hasData && !played.current && motionOn();
  useEffect(() => {
    if (hasData) played.current = true;
  }, [hasData]);
  return enter;
}
