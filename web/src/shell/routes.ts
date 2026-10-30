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
export const ON_WEB: Route[] = ["kanban", "tasks", "releases", "management", "settings"];

/** `#forms` is the debug page, outside the drawer. A view not on the web yet, or anything else, is the board. */
export function routeFromHash(hash: string): Route {
  const name = hash.replace(/^#/, "").split("?")[0];
  if (name === "forms") return "forms";
  return ON_WEB.find((r) => r === name) ?? "kanban";
}

/** The address's hash, kept in step with it: the view, and the view's own place in it (a filter, a tab). */
export function useHash(): string {
  const [hash, setHash] = useState(() => window.location.hash);
  useEffect(() => {
    const changed = () => setHash(window.location.hash);
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return hash;
}

/** The view the address names, so a reload stays on the same view. */
export function useRoute(): Route {
  return routeFromHash(useHash());
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
  const hash = useHash();
  return useMemo(() => tasksRouteFromHash(hash), [hash]);
}

/** Management's tabs, in egui's order, with the id the address carries (`#management?tab=agent-settings`). */
export const MANAGEMENT_TABS = [
  { id: "overview", label: "Overview" },
  { id: "products", label: "Products" },
  { id: "personas", label: "Personas" },
  { id: "instructions", label: "Instructions" },
  { id: "agent-settings", label: "Agent settings" },
  { id: "orchestrators", label: "Orchestrators" },
  { id: "specialists", label: "Specialists" },
  { id: "skills", label: "Skills" },
  { id: "pr-styles", label: "PR styles" },
  { id: "workflow", label: "Workflow" },
] as const;

export type ManagementTab = (typeof MANAGEMENT_TABS)[number]["id"];

/** Settings' tabs, in egui's order (`#settings?tab=log`). */
export const SETTINGS_TABS = [
  { id: "general", label: "General" },
  { id: "daemon", label: "Daemon" },
  { id: "extras", label: "Extras" },
  { id: "log", label: "Log" },
] as const;

export type SettingsTab = (typeof SETTINGS_TABS)[number]["id"];

/** A tabbed view's address: its first tab is the view's own, with no `?tab=`. */
function tabHref<Id extends string>(route: Route, tabs: readonly { id: Id }[], tab: Id): string {
  return tab === tabs[0]!.id ? `#${route}` : `#${route}?tab=${tab}`;
}

/** The tab the address picks; the first, as egui opens on, when it names none or one that does not exist. */
function useTab<Id extends string>(tabs: readonly { id: Id }[]): Id {
  const tab = new URLSearchParams(useHash().split("?")[1] ?? "").get("tab");
  return tabs.find((t) => t.id === tab)?.id ?? tabs[0]!.id;
}

export const managementHref = (tab: ManagementTab = "overview") => tabHref("management", MANAGEMENT_TABS, tab);
export const useManagementTab = () => useTab(MANAGEMENT_TABS);
export const settingsHref = (tab: SettingsTab = "general") => tabHref("settings", SETTINGS_TABS, tab);
export const useSettingsTab = () => useTab(SETTINGS_TABS);

/** Goes to `href` and tells the route hooks at once; `replace` corrects the address without a history entry. */
export function navigate(href: string, replace = false) {
  if (window.location.hash === href) return;
  if (replace) window.history.replaceState(null, "", href);
  else window.history.pushState(null, "", href);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}
