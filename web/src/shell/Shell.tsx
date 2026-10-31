import type { ReactNode, Ref } from "react";

interface Props {
  drawer: ReactNode;
  /** The view's header row. */
  header: ReactNode;
  /** The header's right end, on every view. */
  tools?: ReactNode;
  /** The header's telemetry, at its right end; the board alone has one. */
  readouts?: ReactNode;
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
 * The FUI canvas: the drawer claims the left edge for the window's full height; beside it, on the page colour, the
 * header, the view's body (with the detail panel beside it) and the footer, each an outlined section. Every size is a
 * generated token.
 */
export function Shell({ drawer, header, tools, readouts, footer, panel, dialog, covered, bodyRef, children }: Props) {
  return (
    <div className="shell">
      {drawer}
      <div className="content">
        <header className="view-header outlined">
          {header}
          {readouts}
          {tools}
        </header>
        <div className="body" ref={bodyRef}>
          <main className="view-body outlined" hidden={covered}>
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
