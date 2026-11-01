import type { Agent } from "../api/generated/Agent";

/**
 * Lite's moving status edge (card.rs `edge_motion`), over a card's or fleet row's still one: a visor while the agent
 * works and is not paused (a paused agent is doing nothing), a breath while its tone asks for attention.
 */
export function EdgeMotion({ agent, tone }: { agent: Agent; tone: string }) {
  const motion = agent.status === "working" && !agent.paused ? "visor" : tone === "attention" ? "pulse" : null;
  return motion && <span className="edge-motion" data-motion={motion} aria-hidden="true" />;
}
