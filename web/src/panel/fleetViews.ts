import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { Agent } from "../api/generated/Agent";
import type { FleetView } from "../api/generated/FleetView";
import { title } from "../api/schema";
import type { Notice } from "../shell/messages";
import { groups, taskId } from "../board/groups";

/** A task's fleet view as the panel shows it: the view, or why the first one could not be had. */
export interface ViewLoad {
  view?: FleetView;
  error?: string;
}

/** Lite's `pr_url`: the last gate run's pull request; an earlier run's was replaced. */
export function prUrl(view: FleetView): string | null {
  return view.gate_runs.at(-1)?.pr_url ?? null;
}

/**
 * The fleet views the app has asked for, by task (Lite's `fleet_views`). One request per task at a time: asking again
 * while one is out marks it stale, and another follows when it lands, so an older view never lands last. A failed
 * refresh keeps the view on screen and says so.
 */
export function useFleetViews(client: FleetClient, say: (notice: Notice, text: string) => void) {
  const [views, setViews] = useState<Record<string, ViewLoad>>({});
  const fetching = useRef(new Set<string>());
  const stale = useRef(new Set<string>());
  // The tasks with a view on screen: a failed refresh of one keeps it.
  const shown = useRef(new Set<string>());

  const fetch = useCallback(
    (task: string) => {
      if (fetching.current.has(task)) {
        stale.current.add(task);
        return;
      }
      fetching.current.add(task);
      const landed = () => {
        fetching.current.delete(task);
        if (stale.current.delete(task)) fetch(task);
      };
      client.fleetView(task).then(
        (view) => {
          shown.current.add(task);
          setViews((v) => ({ ...v, [task]: { view } }));
          landed();
        },
        (e: unknown) => {
          const message = e instanceof Error ? e.message : String(e);
          if (shown.current.has(task)) say("warning", `Could not refresh the breakdown: ${message}`);
          else setViews((v) => ({ ...v, [task]: { error: message } }));
          landed();
        },
      );
    },
    [client, say],
  );

  const prs = Object.fromEntries(Object.entries(views).flatMap(([task, l]) => (l.view ? [[task, prUrl(l.view) !== null]] : [])));
  return { views, fetch, prs };
}

/**
 * What the panel adds for an orchestrated task's agent (Lite's `fleet::panel`): an orchestrator's Breakdown, or a
 * fleet agent's link to its task's live lead, which a restart may have replaced.
 */
export function fleetPanel(agents: Agent[], a: Agent): { type: "task" } | { type: "member"; task: string; lead: string } | null {
  if (a.kind === "orchestrator") return { type: "task" };
  if (a.kind !== "specialist") return null;
  const lead = groups(agents).find((g) => taskId(g.lead) === taskId(a))?.lead;
  return lead && lead.id !== a.id ? { type: "member", task: title(lead), lead: lead.id } : null;
}
