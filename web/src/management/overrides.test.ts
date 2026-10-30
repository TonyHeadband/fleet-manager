import { describe, expect, it } from "vitest";
import { editRow, type OverrideDraft } from "./overrides";

const row = (section: string, key: string, value: string, error: string | null = null): OverrideDraft => ({ section, key, value, error });

describe("editRow", () => {
  it("a_new_section_clears_the_key", () => {
    expect(editRow(row("models", "rust", "opus"), { section: "limits" })).toEqual(row("limits", "", "opus"));
  });

  it("a_new_pr_key_clears_the_value", () => {
    expect(editRow(row("pr", "style", "conventional"), { key: "draft" })).toEqual(row("pr", "draft", ""));
    expect(editRow(row("models", "rust", "opus"), { key: "bug-fix" })).toEqual(row("models", "bug-fix", "opus"));
  });

  it("a_change_clears_the_daemon_error_and_no_change_keeps_it", () => {
    expect(editRow(row("models", "ghost", "opus", "gone"), { value: "sonnet" }).error).toBeNull();
    expect(editRow(row("models", "ghost", "opus", "gone"), { value: "opus" }).error).toBe("gone");
  });
});
