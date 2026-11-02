import type { ReactNode } from "react";
import type { WidgetDef } from "./widget";
import "./widgets.css";

/** The frame every widget shares: the title, and the chart block filling the body. */
export function Widget({ def, children }: { def: WidgetDef; children: ReactNode }) {
  return (
    <section className="widget" data-widget={def.id} aria-label={def.title}>
      <h2 className="widget-title">{def.title}</h2>
      <div className="widget-body">{children}</div>
    </section>
  );
}
