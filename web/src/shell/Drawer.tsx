import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { Button } from "../ui/Button";
import type { Load } from "./choices";
import { RadarMark } from "./RadarMark";
import { ON_WEB, VIEWS, type Route } from "./routes";

const NOT_YET = "not on the web yet";

/** "3 agents · 1 needs you", as Lite's `agent_summary` words it. */
export function agentSummary(agents: number, needYou: number) {
  let s = `${agents} ${agents === 1 ? "agent" : "agents"}`;
  if (needYou > 0) s += ` · ${needYou} ${needYou === 1 ? "needs you" : "need you"}`;
  return s;
}

interface Props {
  route: Route;
  connected: boolean;
  agents: number;
  needYou: number;
  personas: Load<Persona[]>;
  persona: string | null;
  onPersona: (id: string) => void;
  products: Load<Product[]>;
  product: string | null;
  onProduct: (id: string) => void;
  /** Why New task cannot start now, or null when it can. */
  newTaskBlocked: string | null;
  onNewTask: () => void;
  /** Where Tasks opens: the filter and selection it had when it was left, as egui keeps them. */
  tasksLink: string;
}

/** Lite's drawer (crates/fleet-gui/src/drawer.rs): fixed width, the window's full height, top to bottom. */
export function Drawer({ route, connected, agents, needYou, personas, persona, onPersona, products, product, onProduct, newTaskBlocked, onNewTask, tasksLink }: Props) {
  const ready = personas.state === "ready" ? personas.value.filter((p) => p.ready) : [];
  const listed = products.state === "ready" ? products.value : [];
  return (
    <nav className="drawer" aria-label="Drawer">
      <div className="drawer-header">
        <span className="app-name">Fleet Manager</span>
      </div>
      <div className="drawer-middle">
        <RadarMark className="watermark" />
        {ready.length > 0 && (
          <div className="drawer-picker">
            <select aria-label="Active persona" value={persona ?? ""} onChange={(e) => onPersona(e.target.value)}>
              {!ready.some((p) => p.id === persona) && <option value="">Pick a persona</option>}
              {ready.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>
        )}
        {listed.length > 0 && (
          <div className="drawer-picker">
            <select aria-label="Active product" value={product ?? ""} onChange={(e) => onProduct(e.target.value)}>
              {product === null && <option value="">Pick a product</option>}
              {listed.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>
        )}
        <div className="drawer-buttons">
          <Button disabled title={NOT_YET}>
            New product
          </Button>
          <Button variant="primary" disabled={newTaskBlocked !== null} title={newTaskBlocked ?? undefined} onClick={onNewTask}>
            New task
          </Button>
        </div>
        <ul className="drawer-nav">
          {VIEWS.map((group, n) => [
            n > 0 && <li key={`sep${n}`} role="separator" />,
            ...group.map((v) =>
              ON_WEB.includes(v.route) ? (
                <li key={v.route}>
                  <a href={v.route === "tasks" ? tasksLink : `#${v.route}`} aria-current={v.route === route ? "page" : undefined}>
                    {v.label}
                  </a>
                </li>
              ) : (
                <li key={v.route}>
                  <a role="link" aria-disabled="true" title={NOT_YET}>
                    {v.label}
                  </a>
                </li>
              ),
            ),
          ])}
        </ul>
      </div>
      <div className="drawer-footer">
        <span className="dot" data-tone={connected ? "done" : "attention"} aria-hidden="true" />
        <span>{connected ? "live" : "reconnecting…"}</span>
        <span className="drawer-summary">{agentSummary(agents, needYou)}</span>
      </div>
    </nav>
  );
}
