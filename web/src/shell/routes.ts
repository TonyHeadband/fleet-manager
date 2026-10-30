import { useEffect, useMemo, useState } from "react";

export type Route = "kanban" | "tasks" | "releases" | "management" | "settings" | "forms";

/** The drawer's views, as Lite lists them: the work, then a separator, then the app's own pages. */
export const VIEWS: { route: Route; label: string }[][] = [
  [
    { route: "kanban", label: "Kanban" },
    { route: "tasks", label: "Tasks" },
    { route: "releases", label: "Releases" },
  ],
  [
    { route: "management", label: "Management" },
    { route: "settings", label: "Settings" },
  ],
];

/** The views the web draws so far; the others show greyed in the drawer until their step lands. */
export const ON_WEB: Route[] = ["kanban", "tasks", "releases"];

/** `#forms` is the debug page, outside the drawer. A view not on the web yet, or anything else, is the board. */
export function routeFromHash(hash: string): Route {
  const name = hash.replace(/^#/, "").split("?")[0];
  if (name === "forms") return "forms";
  return ON_WEB.find((r) => r === name) ?? "kanban";
}

/** The view the address names, kept in step with it, so a reload stays on the same view. */
export function useRoute(): Route {
  const [route, setRoute] = useState(() => routeFromHash(window.location.hash));
  useEffect(() => {
    const changed = () => setRoute(routeFromHash(window.location.hash));
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return route;
}

/** Which rows the Tasks view lists, as egui's `TaskFilter`; each is also the label in "Showing {filter}". */
export type TaskFilter = "all" | "running" | "stopped" | "crashed" | "orphans" | "merged" | "dropped";

const FILTERS: TaskFilter[] = ["all", "running", "stopped", "crashed", "orphans", "merged", "dropped"];

/** The Tasks view on a filter and a task or orphan id, as egui's `open_tasks`: `#tasks?filter=stopped&task=s1`. */
export function tasksHref(filter: TaskFilter = "all", task?: string | null): string {
  const q = new URLSearchParams();
  if (filter !== "all") q.set("filter", filter);
  if (task) q.set("task", task);
  const s = q.toString();
  return s ? `#tasks?${s}` : "#tasks";
}

export function tasksRouteFromHash(hash: string): { filter: TaskFilter; task: string | null } {
  const q = new URLSearchParams(hash.split("?")[1] ?? "");
  return { filter: FILTERS.find((f) => f === q.get("filter")) ?? "all", task: q.get("task") };
}

/** The Tasks view's filter and selection, kept in the address so the strip's counts and Details can link to them. */
export function useTasksRoute(): { filter: TaskFilter; task: string | null } {
  const [hash, setHash] = useState(() => window.location.hash);
  useEffect(() => {
    const changed = () => setHash(window.location.hash);
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return useMemo(() => tasksRouteFromHash(hash), [hash]);
}

/** Goes to `href` and tells the route hooks at once; `replace` corrects the address without a history entry. */
export function navigate(href: string, replace = false) {
  if (window.location.hash === href) return;
  if (replace) window.history.replaceState(null, "", href);
  else window.history.pushState(null, "", href);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}
