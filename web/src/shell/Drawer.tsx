import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { Button } from "../ui/Button";
import { ChoiceMenu } from "../ui/ChoiceMenu";
import type { Load } from "./choices";
import { WATERMARK_DOT_SIZE, WATERMARK_TURN_MS } from "./mark";
import { RadarMark } from "./RadarMark";
import { ON_WEB, VIEWS, type Route } from "./routes";

const NOT_YET = "not on the web yet";

interface Props {
  route: Route;
  connected: boolean;
  personas: Load<Persona[]>;
  persona: string | null;
  onPersona: (id: string) => void;
  products: Load<Product[]>;
  product: string | null;
  onProduct: (id: string) => void;
  /** Why New task cannot start now, or null when it can. */
  newTaskBlocked: string | null;
  onNewTask: () => void;
  /** Opens New product, which egui's drawer always offers. */
  onNewProduct: () => void;
  /** Where a view opens when it keeps its own place (Tasks' filter and selection, Management's tab), as egui does. */
  links: Partial<Record<Route, string>>;
}

/** Lite's drawer (crates/fleet-gui/src/drawer.rs): fixed width, the window's full height, top to bottom. */
export function Drawer({
  route,
  connected,
  personas,
  persona,
  onPersona,
  products,
  product,
  onProduct,
  newTaskBlocked,
  onNewTask,
  onNewProduct,
  links,
}: Props) {
  const ready = personas.state === "ready" ? personas.value.filter((p) => p.ready) : [];
  const listed = products.state === "ready" ? products.value : [];
  return (
    <nav className="drawer" aria-label="Drawer">
      <div className="drawer-header">
        <span className="app-name">Fleet Manager</span>
      </div>
      <div className="drawer-middle">
        <RadarMark className="watermark" spin turnMs={WATERMARK_TURN_MS} dotSize={WATERMARK_DOT_SIZE} />
        {ready.length > 0 && (
          <Picker label="Persona" name="Active persona" items={ready} value={persona} placeholder="Pick a persona" onPick={onPersona} />
        )}
        {listed.length > 0 && (
          <Picker label="Product" name="Active product" items={listed} value={product} placeholder="Pick a product" onPick={onProduct} />
        )}
        <div className="drawer-buttons">
          <Button onClick={onNewProduct}>New product</Button>
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
                  <a className="button ghost" href={links[v.route] ?? `#${v.route}`} aria-current={v.route === route ? "page" : undefined}>
                    {v.label}
                  </a>
                </li>
              ) : (
                <li key={v.route}>
                  <a className="button ghost" role="link" aria-disabled="true" title={NOT_YET}>
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
      </div>
    </nav>
  );
}

/**
 * The drawer's choice of persona or product: the board's filter combo (label, summary, a menu under it) as wide as the
 * drawer, one choice that closes on a pick.
 */
function Picker(props: {
  label: string;
  name: string;
  items: { id: string; name: string }[];
  value: string | null;
  placeholder: string;
  onPick: (id: string) => void;
}) {
  return (
    <div className="drawer-picker">
      <ChoiceMenu width="fill" {...props} />
    </div>
  );
}
