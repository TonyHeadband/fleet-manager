import { render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Card } from "./Card";
import { FleetRow } from "./FleetRow";
import { agent, schema } from "../test/fixtures";

// Lite's moving status edges (card.rs `edge_motion`): a visor runs down and up a working agent's edge, a needs-you
// edge breathes. Still otherwise, and a paused agent is doing nothing.

const motionOf = (el: HTMLElement) => el.querySelector(".edge-motion")?.getAttribute("data-motion") ?? null;
const card = (over: Parameters<typeof agent>[0]) => {
  render(<Card schema={schema} agent={agent(over)} now={1300} onRun={() => {}} onOpen={() => {}} />);
  return screen.getByRole("article");
};

describe("status edge motion", () => {
  it("a_working_card_runs_a_visor", () => {
    expect(motionOf(card({ status: "working", paused: false }))).toBe("visor");
  });

  it("a_paused_working_card_is_still", () => {
    expect(motionOf(card({ status: "working", paused: true }))).toBeNull();
  });

  it("a_needs_you_card_breathes", () => {
    expect(motionOf(card({ status: "needs_you", blocked_reason: "permission", paused: false }))).toBe("pulse");
  });

  it("a_queued_card_is_still", () => {
    expect(motionOf(card({ status: "queued" }))).toBeNull();
  });

  it("a_fleet_row_moves_as_a_card_does", () => {
    render(
      <FleetRow schema={schema} agent={agent({ status: "working", paused: false })} name="rust" now={1300} selected={false} stuck={false} pr={false} onOpen={() => {}} />,
    );
    expect(motionOf(screen.getByRole("button", { name: /rust/ }))).toBe("visor");
  });

  it("the_edges_move_smoothly_under_motion_only", () => {
    const css = readFileSync("src/shell/motion.css", "utf8").replace(/\s+/g, " ");
    const motion = css.slice(css.indexOf("@media (prefers-reduced-motion: no-preference)"));
    expect(motion).toMatch(/\.edge-motion\[data-motion="visor"\]::before \{[^}]*animation: fui-visor var\(--motion-visor\) ease-in-out infinite alternate;/);
    expect(motion).toMatch(/\.edge-motion\[data-motion="pulse"\] \{[^}]*animation: fui-pulse var\(--motion-pulse\) ease-in-out infinite;/);
    expect(css).not.toMatch(/steps\(/);
    // Stopped by Settings → Motion, the band would freeze and the glow stay lit: the mark goes instead.
    expect(css).toMatch(/:root\[data-animations="off"\] \.edge-motion \{ display: none; \}/);
  });
});
