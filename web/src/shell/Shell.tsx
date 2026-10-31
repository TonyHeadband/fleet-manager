import type { ReactNode, Ref } from "react";
import { useScrollFade } from "../ui/scrollFade";

interface Props {
  drawer: ReactNode;
  /** The view's header row. */
  header: ReactNode;
  /** The header's right end, on every view. */
  tools?: ReactNode;
  /** The row under the header; the board alone has one. */
  strip?: ReactNode;
  footer: ReactNode;
  /** The detail panel, beside the view's body. */
  panel?: ReactNode;
  /** A dialog over the whole window. */
  dialog?: ReactNode;
  /** The detail panel covers the view: fullscreen. */
  covered?: boolean;
  /** Measures the width the view and the panel share. */
  bodyRef?: Ref<HTMLDivElement>;
  children: ReactNode;
}

/**
 * Lite's frame: the drawer claims the left edge for the window's full height, then the footer and header their rows
 * in what is left, and the view's body and detail panel share the middle. Every size is a generated token.
 */
export function Shell({ drawer, header, tools, strip, footer, panel, dialog, covered, bodyRef, children }: Props) {
  const fade = useScrollFade<HTMLElement>();
  return (
    <div className="shell">
      {drawer}
      <div className="content">
        <header className="view-header">
          {header}
          {tools}
        </header>
        {strip}
        <div className="body" ref={bodyRef}>
          <main className="view-body scroll-fade" hidden={covered} ref={fade}>
            {children}
          </main>
          {panel}
        </div>
        {footer}
      </div>
      {dialog}
    </div>
  );
}
