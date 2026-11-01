import { useEffect, useMemo, useState } from "react";

export type Route = "kanban" | "tasks" | "releases" | "fleet" | "settings" | "gallery";

/** The drawer's views, as Lite lists them: the work, then a separator, then the app's own pages. */
export const VIEWS: { route: Route; label: string }[][] = [
  [
    { route: "kanban", label: "Kanban" },
    { route: "tasks", label: "Tasks" },
    { route: "releases", label: "Releases" },
  ],
  [
    { route: "fleet", label: "Fleet" },
    { route: "settings", label: "Settings" },
  ],
];

/** The views the web draws so far; the others show greyed in the drawer until their step lands. */
export const ON_WEB: Route[] = ["kanban", "tasks", "releases", "fleet", "settings"];

/** A view not on the web yet, or anything else, is the board. */
export function routeFromHash(hash: string): Route {
  const name = hash.replace(/^#/, "").split("?")[0];
  // Fleet was Management: an address saved before still opens it.
  if (name === "management") return "fleet";
  // The chart catalog is a development aid until the grid editor shows it: a shipped build routes it to the board.
  if (name === "gallery" && import.meta.env.DEV) return "gallery";
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

/** Fleet's tab groups: what the fleet is, the rules its agents work under, and the definitions they run. */
export const FLEET_GROUPS = [
  { id: "fleet", label: "Fleet" },
  { id: "rules", label: "Rules" },
  { id: "definitions", label: "Definitions" },
] as const;

export type FleetGroup = (typeof FLEET_GROUPS)[number]["id"];

/** Fleet's tabs, group by group, with the id the address carries (`#fleet?tab=limits`). */
export const FLEET_TABS = [
  { id: "overview", label: "Overview", group: "fleet" },
  { id: "products", label: "Products", group: "fleet" },
  { id: "personas", label: "Personas", group: "fleet" },
  { id: "agents", label: "Agents", group: "rules" },
  { id: "limits", label: "Limits", group: "rules" },
  { id: "pull-requests", label: "Pull requests", group: "rules" },
  { id: "workflow", label: "Workflow", group: "rules" },
  { id: "instructions", label: "Instructions", group: "definitions" },
  { id: "orchestrators", label: "Orchestrators", group: "definitions" },
  { id: "specialists", label: "Specialists", group: "definitions" },
  { id: "skills", label: "Skills", group: "definitions" },
  { id: "pr-styles", label: "PR styles", group: "definitions" },
] as const;

export type FleetTab = (typeof FLEET_TABS)[number]["id"];

/** Settings' tabs: this machine only, the app and its daemon (`#settings?tab=log`). */
export const SETTINGS_TABS = [
  { id: "app", label: "App" },
  { id: "daemon", label: "Daemon" },
  { id: "tools", label: "Tools" },
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

export const fleetHref = (tab: FleetTab = "overview") => tabHref("fleet", FLEET_TABS, tab);
export const useFleetTab = () => useTab(FLEET_TABS);
export const settingsHref = (tab: SettingsTab = "app") => tabHref("settings", SETTINGS_TABS, tab);
export const useSettingsTab = () => useTab(SETTINGS_TABS);

/** Goes to `href` and tells the route hooks at once; `replace` corrects the address without a history entry. */
export function navigate(href: string, replace = false) {
  if (window.location.hash === href) return;
  if (replace) window.history.replaceState(null, "", href);
  else window.history.pushState(null, "", href);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}
