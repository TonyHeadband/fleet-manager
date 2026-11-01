import { readFileSync } from "node:fs";
import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { fakeNative } from "../test/fixtures";
import { click, openApp } from "../management/harness";

// Settings holds only this machine (the app and its daemon); Fleet holds everything about the fleet, its tabs in
// groups.

const tabs = (list: string) => within(screen.getByRole("tablist", { name: list })).getAllByRole("tab");
const names = (list: string) => tabs(list).map((t) => t.textContent);
const selected = (list: string) => tabs(list).find((t) => t.getAttribute("aria-selected") === "true")?.textContent;
const sections = () => screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);

beforeEach(() => {
  localStorage.clear();
  window.location.hash = "";
});

describe("Settings", () => {
  it("settings_holds_only_this_machine", async () => {
    await openApp("Settings", null, { native: fakeNative() });
    expect(names("Settings tabs")).toEqual(["App", "Daemon", "Tools", "Log"]);
    expect(selected("Settings tabs")).toBe("App");
    for (const s of ["Version", "Shortcuts", "Theme", "Font", "Mono font", "Closing the window", "Footer", "Motion"])
      expect(sections()).toContain(s);
    for (const s of ["Tasks folder", "Limits", "Pull requests", "Agents", "Setup wizard"]) expect(sections()).not.toContain(s);
  });

  it("the_daemon_tab_holds_its_folders_and_the_wizard", async () => {
    await openApp("Settings", "Daemon", { native: fakeNative() });
    await waitFor(() => expect(sections()).toContain("Tasks folder"));
    for (const s of ["Daemon", "Personas folder", "Setup wizard"]) expect(sections()).toContain(s);
    for (const s of ["Limits", "Pull requests", "Agents"]) expect(sections()).not.toContain(s);
  });
});

describe("Fleet", () => {
  it("the_drawer_names_it_fleet", async () => {
    await openApp("Fleet", null);
    expect(within(screen.getByRole("banner")).getByRole("heading", { name: "Fleet" })).toBeInTheDocument();
    expect(within(screen.getByRole("navigation", { name: "Drawer" })).queryByRole("link", { name: "Management" })).toBeNull();
  });

  it("its_tabs_come_in_groups", async () => {
    await openApp("Fleet", null);
    expect(names("Fleet groups")).toEqual(["Fleet", "Rules", "Definitions"]);
    expect(selected("Fleet groups")).toBe("Fleet");
    expect(names("Fleet tabs")).toEqual(["Overview", "Products", "Personas"]);
    await click("Rules");
    expect(names("Fleet tabs")).toEqual(["Agents", "Limits", "Pull requests", "Workflow"]);
    expect(selected("Fleet tabs")).toBe("Agents");
    expect(window.location.hash).toBe("#fleet?tab=agents");
    await click("Definitions");
    expect(names("Fleet tabs")).toEqual(["Instructions", "Specialists", "Skills", "PR styles"]);
  });

  it("the_group_row_reads_as_mono_headings", async () => {
    await openApp("Fleet", null);
    expect(screen.getByRole("tablist", { name: "Fleet groups" })).toHaveClass("tab-groups");
    expect(readFileSync("src/ui/ui.css", "utf8")).toMatch(
      /\.tab-groups \[role="tab"\] \{[^}]*font-family: var\(--font-mono\);[^}]*text-transform: uppercase;/,
    );
  });

  it("the_rules_hold_the_fleet_settings", async () => {
    await openApp("Fleet", "Rules", { native: fakeNative() });
    await waitFor(() => expect(sections()).toContain("Agents"));
    for (const s of ["Allow rules", "Deny rules", "Environment"]) expect(sections()).toContain(s);
    await click("Limits");
    await waitFor(() => expect(sections()).toEqual(["Limits"]));
    await click("Pull requests");
    await waitFor(() => expect(sections()).toEqual(["Pull requests"]));
  });

  it("an_old_management_address_opens_fleet", async () => {
    await openApp("Kanban", null);
    window.location.hash = "#management?tab=personas";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    await waitFor(() => expect(selected("Fleet tabs")).toBe("Personas"));
  });

  describe("tab slides", () => {
    const index = () => document.querySelector<HTMLElement>(".tabbed .pane-track")!.style.getPropertyValue("--index");

    it("settings_tabs_slide", async () => {
      await openApp("Settings", null, { native: fakeNative() });
      expect(index()).toBe("0");
      await click("Log");
      expect(index()).toBe("3");
    });

    it("fleet_tabs_slide_across_their_groups", async () => {
      await openApp("Fleet", null);
      await click("Rules");
      // Agents is the fourth of all Fleet's tabs, whichever group shows it.
      expect(index()).toBe("3");
    });
  });
});
