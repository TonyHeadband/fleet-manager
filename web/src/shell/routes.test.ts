import { afterEach, describe, expect, it, vi } from "vitest";
import { ON_WEB, VIEWS, routeFromHash, supervisorHref } from "./routes";

afterEach(() => vi.unstubAllEnvs());

describe("routeFromHash", () => {
  it("gallery_is_a_route_only_in_dev", () => {
    vi.stubEnv("DEV", true);
    expect(routeFromHash("#gallery")).toBe("gallery");
    vi.stubEnv("DEV", false);
    expect(routeFromHash("#gallery")).toBe("kanban");
  });
});

describe("routes", () => {
  it("supervisor_is_the_drawers_first_view_and_on_the_web", () => {
    expect(VIEWS[0]![0]).toEqual({ route: "supervisor", label: "Supervisor" });
    expect(ON_WEB).toContain("supervisor");
  });
  it("the_hash_names_the_supervisor_page_with_or_without_a_conversation", () => {
    expect(routeFromHash("#supervisor")).toBe("supervisor");
    expect(routeFromHash("#supervisor?c=a1")).toBe("supervisor");
  });
  it("supervisorHref_carries_the_conversation", () => {
    expect(supervisorHref()).toBe("#supervisor");
    expect(supervisorHref("a1")).toBe("#supervisor?c=a1");
  });
});
