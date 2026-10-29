import { describe, expect, it } from "vitest";
import { formRequest, initialValues, isKnown, ready, readyFor } from "./forms";
import { form } from "../test/fixtures";

describe("forms", () => {
  const create = form("release_create");

  it("fills today into a Today default and leaves the rest empty", () => {
    expect(initialValues(create, "2026-10-29")).toEqual({ version: "", start: "2026-10-29", target: "" });
  });

  it("is ready only when every not-blank input has text", () => {
    expect(ready(create, { version: "  ", start: "2026-10-29", target: "" })).toBe(false);
    expect(ready(create, { version: "1.0", start: "2026-10-29", target: "" })).toBe(true);
  });

  it("builds the request from the description, filling and encoding the path", () => {
    expect(formRequest(create, create.submit, { product: "a b" }, { version: " 1.0 ", start: "2026-10-01", target: "soon" })).toEqual({
      method: "POST",
      path: "/api/products/a%20b/releases",
      body: { version: "1.0", start: "2026-10-01", target: "soon" },
    });
  });

  it("knows when a form holds something this build cannot draw", () => {
    expect(isKnown(create)).toBe(true);
    const odd = form("release_create");
    const group = odd.items[0] as { type: "group"; fields: { kind: string }[] };
    group.fields[0]!.kind = "colour";
    expect(isKnown(odd)).toBe(false);
  });

  const breakdown = form("breakdown_decision");
  const [approve, changes] = [breakdown.submit, breakdown.more[0]!];

  it("blocks an action until the inputs it needs have text", () => {
    expect(readyFor(breakdown, changes, { comment: "  " })).toBe(false);
    expect(readyFor(breakdown, changes, { comment: "smaller" })).toBe(true);
    expect(readyFor(breakdown, approve, { comment: "" })).toBe(true);
  });

  it("sends an action's fixed values and the context it names", () => {
    expect(formRequest(breakdown, approve, { id: "t1" }, { comment: "" }, { version: 3 })).toEqual({
      method: "POST",
      path: "/api/tasks/t1/breakdown/decide",
      body: { comment: "", approve: true, version: 3 },
    });
    expect(formRequest(breakdown, approve, { id: "t1" }, { comment: "" }).body).not.toHaveProperty("version");
  });

  it("knows the new kinds and the product's repos as a source", () => {
    expect(isKnown(form("instructions"))).toBe(true);
    expect(isKnown(form("repo_request"))).toBe(true);
    const odd = form("repo_request");
    (odd.items[0] as { source: { type: string } }).source = { type: "calendar" };
    expect(isKnown(odd)).toBe(false);
  });
});
