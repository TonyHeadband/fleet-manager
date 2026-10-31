import type { ReactNode } from "react";
import { ScrollArea } from "./ScrollArea";

/** A view's or tab's scrolling page. */
export function Page({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <ScrollArea className={["tabbed-page", className].filter(Boolean).join(" ")} grow>
      {children}
    </ScrollArea>
  );
}
