import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { fieldOfError, parseDef, writeDef } from "./frontmatter";

const builtin = (path: string) => readFileSync(`../crates/fleet-daemon/builtin/${path}`, "utf8");

const FRONT = `---
# who this is
name: demo
effort: high

summary: "Does: things"
skills:
  - alpha # first
  - 'beta'
model: sonnet
tools:
  allow: ["Bash(ls)"]
---
# Body

text
`;

describe("parseDef", () => {
  it("parses_the_builtin_rust_specialist", () => {
    const { fields, body } = parseDef(builtin("specialists/rust.md"));
    expect(fields).toEqual({
      name: "rust",
      summary: "Rust code - crates, async (tokio), serde, error types, tests - written test first.",
      skills: ["test-driven-development", "systematic-debugging", "verification-before-completion"],
      model: "sonnet",
      mode: "auto",
      effort: "medium",
    });
    expect(body.startsWith("# You are a Rust specialist")).toBe(true);
  });

  it("parses_every_builtin_definition_and_writes_it_back_unchanged", () => {
    for (const dir of ["specialists", "orchestrators"]) {
      for (const f of ["rust", "docs", "reviewer", "test-writer", "typescript-react", "direct", "structured"]) {
        let text: string;
        try {
          text = builtin(`${dir}/${f}.md`);
        } catch {
          continue;
        }
        const d = parseDef(text);
        expect(d.fields.name).toBe(f);
        expect(d.fields.summary).not.toBe("");
        expect(writeDef(text, d)).toBe(text);
      }
    }
  });

  it("reads_quoted_scalars_block_lists_and_ignores_comments", () => {
    const { fields } = parseDef(FRONT);
    expect(fields.summary).toBe("Does: things");
    expect(fields.skills).toEqual(["alpha", "beta"]);
    expect(fields.effort).toBe("high");
  });

  it("an_unparsable_front_yields_empty_fields_and_the_whole_text", () => {
    const d = parseDef("no front here\n");
    expect(d.fields).toEqual({ name: "", summary: "", skills: [], model: null, mode: null, effort: null });
    expect(d.body).toBe("no front here\n");
  });
});

describe("writeDef", () => {
  it("write_with_no_changes_returns_the_original_byte_for_byte", () => {
    expect(writeDef(FRONT, parseDef(FRONT))).toBe(FRONT);
    const crlf = FRONT.replace(/\n/g, "\r\n");
    expect(writeDef(crlf, parseDef(crlf))).toBe(crlf);
  });

  it("round_trip_keeps_comments_order_and_block_lists", () => {
    const d = parseDef(FRONT);
    d.fields.model = "opus";
    expect(writeDef(FRONT, d)).toBe(FRONT.replace("model: sonnet", "model: opus"));
  });

  it("a_changed_block_list_stays_a_block_list", () => {
    const d = parseDef(FRONT);
    d.fields.skills = ["alpha", "gamma"];
    expect(writeDef(FRONT, d)).toContain("skills:\n  - alpha\n  - gamma\nmodel: sonnet");
  });

  it("a_new_value_is_added_and_a_cleared_one_removed", () => {
    const d = parseDef(FRONT);
    d.fields.mode = "manual";
    d.fields.model = null;
    const out = writeDef(FRONT, d);
    expect(out).not.toContain("model:");
    expect(out).toContain("  allow: [\"Bash(ls)\"]\nmode: manual\n---\n");
    expect(out).toContain("# who this is");
    expect(parseDef(out).fields.mode).toBe("manual");
  });

  it("a_summary_needing_quotes_is_quoted", () => {
    const d = parseDef(FRONT);
    d.fields.summary = "a: b # c";
    expect(parseDef(writeDef(FRONT, d)).fields.summary).toBe("a: b # c");
  });

  it("the_body_is_replaced_whole", () => {
    const d = parseDef(FRONT);
    d.body = "new body\n";
    expect(writeDef(FRONT, d)).toBe(FRONT.slice(0, FRONT.indexOf("---\n# Body") + 4) + "new body\n");
  });

  it("a_file_with_no_front_gets_one", () => {
    const d = parseDef("");
    d.fields.name = "x";
    d.fields.summary = "S";
    d.body = "b\n";
    expect(writeDef("", d)).toBe("---\nname: x\nsummary: S\n---\nb\n");
  });
});

describe("block extent and quoting", () => {
  const wrap = (front: string) => `---\n${front}---\nbody\n`;

  it("a_block_scalar_with_a_blank_line_is_read_and_replaced_whole", () => {
    const t = wrap("name: x\nsummary: |\n  p1\n\n  p2\nmodel: opus\n");
    const d = parseDef(t);
    expect(d.fields.summary).toBe("p1\n\np2");
    d.fields.summary = "short";
    expect(writeDef(t, d)).toBe(wrap("name: x\nsummary: short\nmodel: opus\n"));
  });

  it("a_block_list_with_a_blank_or_comment_between_items_is_read_and_replaced_whole", () => {
    const t = wrap("name: x\nskills:\n  - a\n\n# c\n  - b\nmodel: opus\n");
    const d = parseDef(t);
    expect(d.fields.skills).toEqual(["a", "b"]);
    d.fields.skills = ["z"];
    expect(writeDef(t, d)).toBe(wrap("name: x\nskills:\n  - z\nmodel: opus\n"));
  });

  it("trailing_blanks_and_comments_stay_outside_a_rewritten_block", () => {
    const t = wrap("name: x\nmodel: opus\n\n# about mode\nmode: auto\n");
    const d = parseDef(t);
    d.fields.model = "sonnet";
    expect(writeDef(t, d)).toBe(wrap("name: x\nmodel: sonnet\n\n# about mode\nmode: auto\n"));
    d.fields.model = null;
    expect(writeDef(t, d)).toBe(wrap("name: x\n\n# about mode\nmode: auto\n"));
  });

  it("a_multi_line_flow_list_is_read_and_replaced_whole", () => {
    const t = wrap("name: x\nskills: [a,\n  b]\nmodel: opus\n");
    const d = parseDef(t);
    expect(d.fields.skills).toEqual(["a", "b"]);
    d.fields.skills = ["a", "b", "c"];
    expect(writeDef(t, d)).toBe(wrap("name: x\nskills: [a, b, c]\nmodel: opus\n"));
  });

  it("reserved_leading_characters_are_quoted", () => {
    for (const v of ["@reviewer-style checks", "`code` first", ".inf", ".nan", "0x1f", "0o7"]) {
      const d = parseDef(wrap("name: x\n"));
      d.fields.summary = v;
      const out = writeDef(wrap("name: x\n"), d);
      expect(out).toContain(`summary: "`);
      expect(parseDef(out).fields.summary).toBe(v);
    }
  });

  it("flow_list_items_with_separators_are_quoted", () => {
    const d = parseDef(wrap("name: x\nskills: [a]\n"));
    d.fields.skills = ["a,b", "c]", "{d"];
    const out = writeDef(wrap("name: x\nskills: [a]\n"), d);
    expect(parseDef(out).fields.skills).toEqual(["a,b", "c]", "{d"]);
  });

  it("a_duplicate_key_is_read_and_edited_at_its_first_occurrence", () => {
    const t = wrap("name: x\nmodel: a\nmodel: b\n");
    const d = parseDef(t);
    expect(d.fields.model).toBe("a");
    d.fields.model = "c";
    expect(writeDef(t, d)).toBe(wrap("name: x\nmodel: c\nmodel: b\n"));
  });
});

describe("fieldOfError", () => {
  it("field_of_error_maps_the_daemons_messages", () => {
    expect(fieldOfError("specialists/x.md: skill `x` not found")).toBe("skills");
    expect(fieldOfError("model must be a name such as sonnet or claude-sonnet-5-5")).toBe("model");
    expect(fieldOfError("x.md: `summary` is empty")).toBe("summary");
    expect(fieldOfError("x.md: mode plan is not allowed (auto, acceptEdits, manual, dontAsk)")).toBe("mode");
    expect(fieldOfError("x.md: effort huge is not allowed (low, medium, high, xhigh, max)")).toBe("effort");
    expect(fieldOfError("x.md: unknown key `foo`")).toBeNull();
    expect(fieldOfError("disk full")).toBeNull();
  });
});
