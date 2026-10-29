import { useEffect, useState } from "react";

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
export const ON_WEB: Route[] = ["kanban"];

/** `#forms` is the debug page, outside the drawer. A view not on the web yet, or anything else, is the board. */
export function routeFromHash(hash: string): Route {
  const name = hash.replace(/^#/, "");
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
