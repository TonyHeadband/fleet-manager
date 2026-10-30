import { describe, expect, it } from "vitest";
import { agent, workspace } from "../test/fixtures";
import { isLoopbackHost, panelPath, terminalBlocked } from "./header";

// Ported from crates/fleet-gui/tests/panels.rs and app.rs; the names are Lite's.
describe("panel header rules", () => {
  it("header_shows_repo_and_branch_path", () => {
    expect(panelPath(agent())).toBe("core / fleet/fix-the-tests");
    const two = agent({ workspaces: [{ ...workspace("core"), branch: "fleet/x" }, { ...workspace("portal"), branch: "fleet/x" }] });
    expect(panelPath(two), "every repo, the shared branch once").toBe("core, portal / fleet/x");
  });

  it("the_panel_path_is_the_task_folder_until_a_repo_arrives", () => {
    expect(panelPath(agent({ workspaces: [], workdir: "/tasks/no-repo-yet" }))).toBe("/tasks/no-repo-yet");
  });

  it("open_in_terminal_is_disabled_when_stopped", () => {
    expect(terminalBlocked(agent({ status: "stopped", attach: ["fleet-attach", "a1"] }), "127.0.0.1")).toBe("the agent is stopped");
    expect(terminalBlocked(agent({ attach: null }), "127.0.0.1"), "no attach command").toBe("the agent is stopped");
  });

  it("the_browser_says_why_it_cannot_open_a_terminal", () => {
    const running = agent({ attach: ["fleet-attach", "a1"] });
    expect(terminalBlocked(running, "fleet.lan")).toBe("the daemon runs on another machine");
    expect(terminalBlocked(running, "localhost")).toBe("the browser cannot open a terminal");
  });

  it("loopback_hosts_are_lites", () => {
    for (const h of ["localhost", "LOCALHOST", "127.0.0.1", "127.3.4.5", "::1", "[::1]"]) expect(isLoopbackHost(h), h).toBe(true);
    for (const h of ["10.0.0.2", "fleet.lan", "128.0.0.1", ""]) expect(isLoopbackHost(h), h).toBe(false);
  });
});
