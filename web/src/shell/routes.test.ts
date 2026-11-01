import { afterEach, describe, expect, it, vi } from "vitest";
import { routeFromHash } from "./routes";

afterEach(() => vi.unstubAllEnvs());

describe("routeFromHash", () => {
  it("gallery_is_a_route_only_in_dev", () => {
    vi.stubEnv("DEV", true);
    expect(routeFromHash("#gallery")).toBe("gallery");
    vi.stubEnv("DEV", false);
    expect(routeFromHash("#gallery")).toBe("kanban");
  });
});
