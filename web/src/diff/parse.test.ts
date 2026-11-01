import { describe, expect, it } from "vitest";
import { parseDiff, splitRows } from "./parse";

const modified = [
  "diff --git a/src/store.rs b/src/store.rs",
  "index 1111111..2222222 100644",
  "--- a/src/store.rs",
  "+++ b/src/store.rs",
  "@@ -10,3 +10,4 @@ fn backfill_tasks",
  " for a in agents {",
  "-    if a.session.is_some() {",
  "+    let owns = a.kind.owns_task();",
  "+    if owns && a.session.is_some() {",
  "     self.insert_task(a)?;",
].join("\n");

describe("parseDiff", () => {
  it("parses_a_modified_file_with_line_numbers", () => {
    const [f] = parseDiff(modified, false);
    expect(f).toMatchObject({ path: "src/store.rs", oldPath: null, status: "M", binary: false, added: 2, removed: 1, cut: false });
    expect(f!.hunks).toHaveLength(1);
    const lines = f!.hunks[0]!.lines;
    expect(lines[0]).toEqual({ kind: "ctx", text: "for a in agents {", oldNo: 10, newNo: 10 });
    expect(lines[1]).toEqual({ kind: "del", text: "    if a.session.is_some() {", oldNo: 11, newNo: null });
    expect(lines[2]).toEqual({ kind: "add", text: "    let owns = a.kind.owns_task();", oldNo: null, newNo: 11 });
    expect(lines[4]).toEqual({ kind: "ctx", text: "    self.insert_task(a)?;", oldNo: 12, newNo: 13 });
  });

  it("marks_added_deleted_and_renamed_files", () => {
    const text = [
      "diff --git a/new.rs b/new.rs",
      "new file mode 100644",
      "--- /dev/null",
      "+++ b/new.rs",
      "@@ -0,0 +1,1 @@",
      "+fn main() {}",
      "diff --git a/gone.md b/gone.md",
      "deleted file mode 100644",
      "--- a/gone.md",
      "+++ /dev/null",
      "@@ -1,1 +0,0 @@",
      "-old",
      "diff --git a/x.rs b/y.rs",
      "similarity index 100%",
      "rename from x.rs",
      "rename to y.rs",
    ].join("\n");
    const [added, deleted, renamed] = parseDiff(text, false);
    expect(added).toMatchObject({ path: "new.rs", status: "A", added: 1, removed: 0 });
    expect(deleted).toMatchObject({ path: "gone.md", status: "D", added: 0, removed: 1 });
    expect(renamed).toMatchObject({ path: "y.rs", oldPath: "x.rs", status: "R", hunks: [] });
  });

  it("reads_paths_with_spaces_as_git_writes_them", () => {
    // git does not quote a space; it ends the `---`/`+++` path with a tab.
    const text = ["diff --git a/dir/a b.txt b/dir/a b.txt", "--- a/dir/a b.txt\t", "+++ b/dir/a b.txt\t", "@@ -1 +1 @@", "-x", "+y"].join("\n");
    expect(parseDiff(text, false)[0]!.path).toBe("dir/a b.txt");
  });

  it("unquotes_octal_escaped_paths", () => {
    // core.quotePath's default: non-ASCII bytes as octal escapes inside quotes.
    const text = ['diff --git "a/caf\\303\\251.txt" "b/caf\\303\\251.txt"', '--- "a/caf\\303\\251.txt"', '+++ "b/caf\\303\\251.txt"', "@@ -1 +1 @@", "-x", "+y"].join("\n");
    expect(parseDiff(text, false)[0]!.path).toBe("café.txt");
  });

  it("unquotes_renamed_paths", () => {
    const text = ['diff --git "a/caf\\303\\251.txt" b/cafe.txt', "similarity index 100%", 'rename from "caf\\303\\251.txt"', "rename to cafe.txt"].join("\n");
    expect(parseDiff(text, false)[0]).toMatchObject({ status: "R", oldPath: "café.txt", path: "cafe.txt" });
  });

  it("keeps_a_badly_quoted_path_rather_than_failing", () => {
    const text = ['diff --git a/x b/x', '--- "a/bad\\q.txt"', '+++ "b/bad\\q.txt"', "@@ -1 +1 @@", "-x", "+y"].join("\n");
    expect(() => parseDiff(text, false)).not.toThrow();
  });

  it("marks_empty_added_and_deleted_files", () => {
    // An empty file has no ---/+++ lines; only the mode line says what happened.
    const text = ["diff --git a/pkg/__init__.py b/pkg/__init__.py", "new file mode 100644", "index 0000000..e69de29", "diff --git a/.gitkeep b/.gitkeep", "deleted file mode 100644", "index e69de29..0000000"].join("\n");
    const [added, deleted] = parseDiff(text, false);
    expect(added).toMatchObject({ path: "pkg/__init__.py", status: "A" });
    expect(deleted).toMatchObject({ path: ".gitkeep", status: "D" });
  });

  it("marks_binary_files", () => {
    const text = ["diff --git a/img.png b/img.png", "index 1..2 100644", "Binary files a/img.png and b/img.png differ"].join("\n");
    expect(parseDiff(text, false)[0]).toMatchObject({ path: "img.png", status: "M", binary: true, hunks: [] });
  });

  it("ignores_no_newline_markers", () => {
    const text = ["diff --git a/a b/a", "--- a/a", "+++ b/a", "@@ -1 +1 @@", "-x", "\\ No newline at end of file", "+y"].join("\n");
    expect(parseDiff(text, false)[0]!.hunks[0]!.lines.map((l) => l.kind)).toEqual(["del", "add"]);
  });

  it("marks_the_last_file_cut_when_truncated", () => {
    const text = `${modified}\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,9 +1,9 @@\n ctx\n-old`;
    const files = parseDiff(text, true);
    expect(files.map((f) => f.cut)).toEqual([false, true]);
    expect(files[1]!.hunks[0]!.lines).toHaveLength(2);
  });

  it("is_empty_for_an_empty_diff", () => {
    expect(parseDiff("", false)).toEqual([]);
  });
});

describe("splitRows", () => {
  it("splits_a_hunk_into_aligned_rows", () => {
    const text = ["diff --git a/a b/a", "--- a/a", "+++ b/a", "@@ -1,4 +1,3 @@", " c1", "-d1", "-d2", "+a1", " c2"].join("\n");
    const rows = splitRows(parseDiff(text, false)[0]!.hunks[0]!);
    expect(rows.map((r) => [r.left?.text ?? null, r.right?.text ?? null])).toEqual([
      ["c1", "c1"],
      ["d1", "a1"],
      ["d2", null],
      ["c2", "c2"],
    ]);
  });
});
