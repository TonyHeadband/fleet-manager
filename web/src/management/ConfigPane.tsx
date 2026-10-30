import type { ReactNode } from "react";
import { ErrorLine } from "../ui/ErrorLine";

/** egui's `config_pane`, which both config tabs share: "Loading…" until the first answer, the load error, then the
 * tab at its width. */
export function ConfigPane({
  answered,
  loadError,
  width,
  children,
}: {
  answered: boolean;
  loadError: string | null;
  width: "editor" | "settings";
  children: ReactNode;
}) {
  return (
    <div className="management-page">
      {!answered ? (
        <p className="note">Loading…</p>
      ) : (
        <div className="config-pane" data-width={width}>
          {loadError !== null && <ErrorLine text={loadError} />}
          {children}
        </div>
      )}
    </div>
  );
}
