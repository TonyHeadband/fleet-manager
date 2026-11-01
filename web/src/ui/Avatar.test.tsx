import { render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Avatar } from "./Avatar";
import { FIGURE, supervisorSeed } from "./avatarArt";

const css = readFileSync("src/ui/ui.css", "utf8");

describe("Avatar", () => {
  it("an_avatar_without_a_seed_is_the_empty_frame", () => {
    const { container } = render(
      <>
        <Avatar title="Ada" />
        <Avatar seed="" />
      </>,
    );
    const [named, blank] = container.querySelectorAll("span.avatar");
    expect(named).toHaveAttribute("title", "Ada");
    expect(named).toHaveAttribute("aria-hidden", "true");
    expect(named!.querySelector("svg")).toBeNull();
    expect(blank!.querySelector("svg")).toBeNull();
  });

  it("a_seeded_avatar_draws_in_a_30_by_40_box", () => {
    const { container } = render(<Avatar title="Ada" seed="ada" />);
    const span = container.querySelector("span.avatar")!;
    expect(span).toHaveAttribute("aria-hidden", "true");
    expect(span).toHaveAttribute("title", "Ada");
    expect(span).toHaveAttribute("data-kind", "persona");
    expect(span).toHaveAttribute("data-seed", "ada");
    expect(span.querySelector("svg")).toHaveAttribute("viewBox", "0 0 30 40");
  });

  it("a_persona_avatar_is_clipped_to_the_figure", () => {
    const { container } = render(<Avatar seed="ada" />);
    const svg = container.querySelector("svg")!;
    const figure = [...svg.querySelectorAll("clipPath")].find((c) => c.querySelectorAll("circle").length === 2)!;
    const circles = [...figure.querySelectorAll("circle")].map((c) => [c.getAttribute("cx"), c.getAttribute("cy"), c.getAttribute("r")].map(Number));
    expect(circles).toEqual([
      [FIGURE.head.cx, FIGURE.head.cy, FIGURE.head.r],
      [FIGURE.shoulders.cx, FIGURE.shoulders.cy, FIGURE.shoulders.r],
    ]);
    const scan = svg.querySelector(`g[clip-path="url(#${figure.id})"]`)!;
    expect(scan.querySelectorAll("line").length).toBeGreaterThan(0);
    const outline = [...svg.querySelectorAll("circle")].filter((c) => !figure.contains(c));
    expect(outline).toHaveLength(2);
    for (const c of outline) expect(scan.compareDocumentPosition(c) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("a_supervisor_avatar_has_a_square_head_and_the_shoulders", () => {
    const { container } = render(<Avatar kind="supervisor" seed={supervisorSeed("work")} />);
    const svg = container.querySelector("svg")!;
    const figure = [...svg.querySelectorAll("clipPath")].find((c) => c.querySelector("rect") && c.querySelector("circle"))!;
    const side = FIGURE.head.r * 2;
    const head = figure.querySelector("rect")!;
    expect(["x", "y", "width", "height"].map((a) => Number(head.getAttribute(a)))).toEqual([FIGURE.head.cx - FIGURE.head.r, FIGURE.head.cy - FIGURE.head.r, side, side]);
    const shoulders = figure.querySelector("circle")!;
    expect(["cx", "cy", "r"].map((a) => Number(shoulders.getAttribute(a)))).toEqual([FIGURE.shoulders.cx, FIGURE.shoulders.cy, FIGURE.shoulders.r]);
    const scan = svg.querySelector(`g[clip-path="url(#${figure.id})"]`)!;
    expect(scan.querySelectorAll("line").length).toBeGreaterThan(0);
    // The outline: a square head and the shoulders, drawn over the scan lines.
    expect([...svg.querySelectorAll("rect")].filter((r) => !svg.querySelector("defs")!.contains(r))).toHaveLength(1);
    expect([...svg.querySelectorAll("circle")].filter((c) => !figure.contains(c))).toHaveLength(1);
  });

  it("scan_lines_keep_butt_caps_so_their_gaps_stay_open", () => {
    // A square cap adds half the weight at each end, and a 1.1 row would close the 0.8 breaks the owner tuned.
    const { container } = render(<Avatar seed="ada" />);
    for (const l of container.querySelectorAll("svg line")) expect(l.getAttribute("stroke-linecap") ?? "butt").toBe("butt");
  });

  it("traces_have_square_caps_and_miter_joins", () => {
    const { container } = render(<Avatar kind="specialist" seed="rust" />);
    for (const p of container.querySelectorAll("svg polyline")) {
      expect(p).toHaveAttribute("stroke-linecap", "square");
      expect(p).toHaveAttribute("stroke-linejoin", "miter");
    }
  });

  it("a_specialist_avatar_draws_its_traces", () => {
    const { container } = render(<Avatar kind="specialist" seed="rust" />);
    expect(container.querySelector("span.avatar")).toHaveAttribute("data-kind", "specialist");
    expect(container.querySelectorAll("svg polyline").length).toBeGreaterThan(0);
    expect(container.querySelectorAll("svg line")).toHaveLength(0);
  });

  it("two_avatars_never_share_a_clip_id", () => {
    const { container } = render(
      <>
        <Avatar seed="ada" />
        <Avatar seed="ada" />
      </>,
    );
    const svgs = [...container.querySelectorAll("svg")];
    expect(svgs).toHaveLength(2);
    const ids = svgs.flatMap((s) => [...s.querySelectorAll("clipPath")].map((c) => c.id));
    expect(new Set(ids).size).toBe(ids.length);
    for (const s of svgs) {
      const own = new Set([...s.querySelectorAll("clipPath")].map((c) => c.id));
      for (const g of s.querySelectorAll("[clip-path]")) expect(own.has(g.getAttribute("clip-path")!.slice(5, -1))).toBe(true);
    }
  });

  it("colours_come_from_the_theme_variables", () => {
    const { container } = render(
      <>
        <Avatar seed="ada" />
        <Avatar kind="specialist" seed="rust" />
      </>,
    );
    const strokes = [...container.querySelectorAll<SVGElement>("line, polyline, circle:not(clipPath circle)")].map((e) => e.style.stroke);
    expect(strokes.length).toBeGreaterThan(0);
    for (const s of strokes) {
      expect(s).toMatch(/^oklch\(var\(--avatar-l2?\) var\(--avatar-c\) \d+\)$/);
      expect(s).not.toContain("#");
    }
  });

  it("the_avatar_svg_fills_its_frame", () => {
    expect(css).toMatch(/\.avatar svg \{[^}]*width: 100%[^}]*height: 100%/);
    expect(css).not.toMatch(/aspect-ratio/);
  });
});
