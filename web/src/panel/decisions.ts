import { useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { FleetView } from "../api/generated/FleetView";
import type { Notice } from "../shell/messages";
import type { ViewLoad } from "./fleetViews";

/**
 * The owner's breakdown decisions, by task (Lite's `decide_breakdown`). One goes at a time per task, for the version
 * on screen; once sent, the controls wait for the view that shows it, since deciding the old one again is refused.
 */
export function useBreakdownDecisions(client: FleetClient, views: Record<string, ViewLoad>, fetch: (task: string) => void, say: (notice: Notice, text: string) => void) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [deciding, setDeciding] = useState<ReadonlySet<string>>(new Set());
  // The view each decided task showed when its decision went: a newer view lifts the wait.
  const [decided, setDecided] = useState<Record<string, FleetView>>({});
  // Read before a render catches up, so a double press sends once.
  const inFlight = useRef(new Set<string>());

  const shownView = (task: string) => views[task]?.view;
  const busy = (task: string) => deciding.has(task) || (decided[task] !== undefined && decided[task] === shownView(task));

  const decide = (task: string, comment: string | null) => {
    const view = shownView(task);
    const version = view?.breakdowns.at(-1)?.version;
    if (inFlight.current.has(task) || busy(task) || version === undefined) return;
    inFlight.current.add(task);
    setDeciding(new Set(inFlight.current));
    client.decideBreakdown(task, { approve: comment === null, comment, version }).then(
      () => {
        setDecided((d) => ({ ...d, [task]: view! }));
        setDrafts(({ [task]: _, ...rest }) => rest);
        say("success", comment === null ? "Breakdown approved" : "Changes requested");
      },
      (e: unknown) => say("error", e instanceof Error ? e.message : String(e)),
    ).finally(() => {
      inFlight.current.delete(task);
      setDeciding(new Set(inFlight.current));
      fetch(task);
    });
  };

  return {
    busy,
    draft: (task: string) => drafts[task] ?? "",
    setDraft: (task: string, text: string) => setDrafts((d) => ({ ...d, [task]: text })),
    decide,
  };
}
