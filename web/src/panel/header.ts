import type { Agent } from "../api/generated/Agent";

// Lite's panel header rules (crates/fleet-gui/src/app.rs `panel_path`, `terminal_blocked`).

/** `repos / branch`: every workspace's repo, and the branch once when they share it; the task folder until a repo
 * arrives. */
export function panelPath(a: Agent): string {
  if (a.workspaces.length === 0) return a.workdir;
  const repos = a.workspaces.map((w) => w.repo).join(", ");
  const branches = [...new Set(a.workspaces.map((w) => w.branch))].join(", ");
  return `${repos} / ${branches}`;
}

/** Lite's `login::is_loopback_url`, for a host name alone. */
export function isLoopbackHost(host: string): boolean {
  const h = host.replace(/^\[(.*)\]$/, "$1").toLowerCase();
  return h === "localhost" || h === "::1" || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h);
}

/**
 * Why Open in terminal is off, in Lite's order. The browser cannot start a program, so on the web the last reason
 * always holds; a shell that can (the Tauri app) supplies Lite's own last checks instead.
 */
export function terminalBlocked(a: Agent, daemonHost: string): string {
  if (a.status === "stopped" || !a.attach) return "the agent is stopped";
  if (!isLoopbackHost(daemonHost)) return "the daemon runs on another machine";
  return "the browser cannot open a terminal";
}
