import type { ReactNode } from "react";

/** A titled section under egui's mono uppercase caption (`section`, `caption`). `aside` sits in the heading row. */
export function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="section">
      {aside === undefined ? (
        <h3 className="mono-label caption">{title}</h3>
      ) : (
        <div className="section-head">
          <h3 className="mono-label caption">{title}</h3>
          {aside}
        </div>
      )}
      {children}
    </section>
  );
}
