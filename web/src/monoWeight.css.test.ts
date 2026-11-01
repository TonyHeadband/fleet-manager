import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Read from disk (vitest runs in web/): vitest turns a stylesheet import into an empty string.
const sheets = (dir: string): string[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? sheets(join(dir, e.name)) : e.name.endsWith(".css") ? [join(dir, e.name)] : [],
  );

describe("mono weight", () => {
  it("every_rule_in_the_mono_font_sets_a_mono_weight", () => {
    const files = sheets("src");
    expect(files.length).toBeGreaterThan(10);
    const bad: string[] = [];
    for (const file of files) {
      for (const [, selector = "", body = ""] of readFileSync(file, "utf8").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
        if (!/font-family:\s*var\(--font-mono\)/.test(body)) continue;
        if (!/font-weight:\s*var\(--weight-mono(-strong)?\)/.test(body)) bad.push(`${file}: ${selector.trim()}`);
      }
    }
    expect(bad).toEqual([]);
  });
});
