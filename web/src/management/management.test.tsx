import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { click, countingTasks, management, managedTask, twoProducts } from "./harness";

// Ports of the frame, Overview and Workflow tests in crates/fleet-gui/tests/management.rs.

const tabs = () => screen.getAllByRole("tab").map((t) => t.textContent);
const selectedTab = () => screen.getByRole("tab", { selected: true }).textContent;

beforeEach(() => localStorage.clear());

describe("Management frame", () => {
  it("nav_lists_management_above_settings", async () => {
    await management("Overview");
    const links = within(screen.getByRole("navigation", { name: "Drawer" }))
      .getAllByRole("link")
      .map((l) => l.textContent);
    const at = (n: string) => links.indexOf(n);
    expect(at("Kanban")).toBeLessThan(at("Tasks"));
    expect(at("Tasks")).toBeLessThan(at("Management"));
    expect(at("Management")).toBeLessThan(at("Settings"));
  });

  it("management_opens_on_overview_and_remembers_its_tab", async () => {
    await management("Overview");
    expect(within(screen.getByRole("banner")).getByRole("heading", { name: "Management" })).toBeInTheDocument();
    expect(selectedTab()).toBe("Overview");
    for (const t of ["Overview", "Products", "Personas", "Instructions", "Agent settings"]) expect(tabs()).toContain(t);
    await click("Instructions");
    await click("Kanban");
    await click("Management");
    expect(selectedTab()).toBe("Instructions");
  });

  it("overview_is_the_first_tab", async () => {
    await management("Overview", { products: twoProducts() });
    expect(selectedTab()).toBe("Overview");
    expect(tabs()).toEqual([
      "Overview",
      "Products",
      "Personas",
      "Instructions",
      "Agent settings",
      "Orchestrators",
      "Specialists",
      "Skills",
      "PR styles",
      "Workflow",
    ]);
  });

  it("workflow_tab_explains_the_file", async () => {
    await management("Workflow");
    expect(selectedTab()).toBe("Workflow");
    expect(screen.getByText(/Each repo lists its checks in \.fleet\/workflow\.yml on its base branch\./)).toBeInTheDocument();
    expect(screen.getByText(/steps:/)).toBeInTheDocument();
    expect(screen.getByText("An editor for this file is coming. For now, add the file to the repo and push it.")).toBeInTheDocument();
  });
});

describe("Overview", () => {
  const card = (name: string) => screen.getByRole("article", { name });

  it("overview_counts_per_product", async () => {
    const [tasks, agents] = countingTasks();
    await management("Overview", { tasks, agents });
    await screen.findByRole("article", { name: "Alpha" });
    // Alpha: two working, one needing the user, nothing in review, one merged.
    for (const text of ["Working 2", "Needs you 1", "Review 0", "Merged 1"])
      expect(within(card("Alpha")).getByText(text)).toBeInTheDocument();
    // Beta: one working, two in review, nothing else.
    for (const text of ["Working 1", "Needs you 0", "Review 2", "Merged 0"])
      expect(within(card("Beta")).getByText(text)).toBeInTheDocument();
    // Only the unreleased release, with both bars as the Releases view draws them.
    expect(within(card("Alpha")).getByText("1.0")).toBeInTheDocument();
    expect(within(card("Alpha")).getByText("1/2 tasks")).toBeInTheDocument();
    expect(within(card("Alpha")).getByText("11/32 days")).toBeInTheDocument();
    expect(screen.queryByText("0.9")).toBeNull();
    // A count of nothing is drawn low; one with tasks in its tone.
    expect(within(card("Alpha")).getByText("Review 0")).not.toHaveAttribute("data-tone");
    expect(within(card("Alpha")).getByText("Working 2")).toHaveAttribute("data-tone", "active");
  });

  it("overview_product_filter", async () => {
    const [tasks, agents] = countingTasks();
    await management("Overview", { tasks, agents });
    expect(await screen.findByRole("button", { name: "Alpha" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Beta" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Product" }));
    await userEvent.click(screen.getByRole("button", { name: "Show Beta" }));
    expect(screen.queryByRole("button", { name: "Alpha" })).toBeNull();
    expect(screen.getByText("Review 2")).toBeInTheDocument();
    expect(screen.queryByText("Review 0")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Product" }));
    await userEvent.click(screen.getByRole("button", { name: "Show all products" }));
    expect(screen.getByRole("button", { name: "Alpha" })).toBeInTheDocument();
  });

  it("clicking_a_product_name_makes_it_active_and_nothing_else", async () => {
    const [tasks, agents] = countingTasks();
    const client = await management("Overview", { tasks, agents });
    await userEvent.click(await screen.findByRole("button", { name: "Beta" }));
    expect(screen.getByRole("combobox", { name: "Active product" })).toHaveValue("b");
    expect(localStorage.getItem("fleet.product")).toBe("b");
    expect(within(card("Beta")).getByText("active")).toBeInTheDocument();
    expect(client.setTaskRelease).not.toHaveBeenCalled();
    expect(selectedTab()).toBe("Overview");
  });

  it("overview_counts_a_fleet_by_its_most_urgent_agent", async () => {
    await management("Overview", {
      tasks: [managedTask("f1", "a"), managedTask("t1", "a")],
      agents: [
        { id: "f1", product: "a", status: "review", kind: "orchestrator" },
        { id: "f1-s", product: "a", status: "needs_you", task: "f1" },
        { id: "t1", product: "a", status: "talking" },
      ],
    });
    await screen.findByRole("article", { name: "Alpha" });
    // Not Review: the orchestrator alone would be, but its specialist needs the user.
    for (const text of ["Working 1", "Needs you 1"]) expect(within(card("Alpha")).getByText(text)).toBeInTheDocument();
  });

  it("no_products_says_so", async () => {
    await management("Overview", { products: [] });
    expect(await screen.findByText("No products yet")).toBeInTheDocument();
  });
});
