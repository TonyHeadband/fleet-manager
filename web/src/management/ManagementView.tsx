import type { ReactNode } from "react";
import { MANAGEMENT_TABS, managementHref, type ManagementTab } from "../shell/routes";
import { TabbedView } from "../shell/TabbedView";

// A port of crates/fleet-gui/src/views/management/mod.rs: one tab per area of the fleet's configuration.

export function ManagementView({ tab, body }: { tab: ManagementTab; body: (tab: ManagementTab) => ReactNode }) {
  return <TabbedView className="management" tabs={MANAGEMENT_TABS} selected={tab} href={managementHref} body={body} />;
}
