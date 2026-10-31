import type { ComponentProps } from "react";
import { DaemonSections } from "../settings/DaemonSections";

/** A Fleet rules tab that is the daemon's own settings (Limits, Pull requests), laid out as Settings lays them. */
export function RulesPage(props: ComponentProps<typeof DaemonSections>) {
  return (
    <div className="tabbed-page settings-page">
      <DaemonSections {...props} />
    </div>
  );
}
