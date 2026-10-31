import type { ReactNode } from "react";
import { useScrollFade } from "./scrollFade";

/** A view's or tab's scrolling page, its edges fading where it goes on. */
export function Page({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={["tabbed-page scroll-fade", className].filter(Boolean).join(" ")} ref={useScrollFade<HTMLDivElement>()}>
      {children}
    </div>
  );
}
