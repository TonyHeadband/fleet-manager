import type { ReactNode } from "react";

interface Props {
  drawer: ReactNode;
  /** The view's header row. */
  header: ReactNode;
  /** The row under the header; the board alone has one. */
  strip?: ReactNode;
  footer: ReactNode;
  /** The detail panel, beside the view's body. */
  panel?: ReactNode;
  children: ReactNode;
}

/**
 * Lite's frame: the drawer claims the left edge for the window's full height, then the footer and header their rows
 * in what is left, and the view's body and detail panel share the middle. Every size is a generated token.
 */
export function Shell({ drawer, header, strip, footer, panel, children }: Props) {
  return (
    <div className="shell">
      {drawer}
      <div className="content">
        <header className="view-header">{header}</header>
        {strip}
        <div className="body">
          <main className="view-body">{children}</main>
          {panel}
        </div>
        {footer}
      </div>
    </div>
  );
}
