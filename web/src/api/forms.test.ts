import { describe, expect, it } from "vitest";
import { formRequest, initialValues, isKnown, ready } from "./forms";
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
    expect(formRequest(create, { product: "a b" }, { version: " 1.0 ", start: "2026-10-01", target: "soon" })).toEqual({
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
});
