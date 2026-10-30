import type { ReactNode } from "react";

/** A titled section under egui's mono uppercase caption (`section`, `caption`). */
export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="section">
      <h3 className="mono-label">{title}</h3>
      {children}
    </section>
  );
}
