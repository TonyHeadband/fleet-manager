import type { ReactNode } from "react";
import type { WidgetDef } from "./widget";
import "./widgets.css";

/** The frame every widget shares: title, the chart block filling the body, and the source. */
export function Widget({ def, children }: { def: WidgetDef; children: ReactNode }) {
  return (
    <section className="widget" data-widget={def.id} aria-label={def.title}>
      <h2 className="widget-title">{def.title}</h2>
      <div className="widget-body">{children}</div>
      <footer className="widget-source">{def.source}</footer>
    </section>
  );
}
