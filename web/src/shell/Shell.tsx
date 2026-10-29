import type { ReactNode, Ref } from "react";

interface Props {
  drawer: ReactNode;
  /** The view's header row. */
  header: ReactNode;
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
export function Shell({ drawer, header, strip, footer, panel, dialog, covered, bodyRef, children }: Props) {
  return (
    <div className="shell">
      {drawer}
      <div className="content">
        <header className="view-header">{header}</header>
        {strip}
        <div className="body" ref={bodyRef}>
          <main className="view-body" hidden={covered}>
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
