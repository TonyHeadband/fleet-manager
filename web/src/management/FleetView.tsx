import type { ReactNode } from "react";
import { FLEET_GROUPS, FLEET_TABS, fleetHref, type FleetTab } from "../shell/routes";
import { TabbedView } from "../shell/TabbedView";

// Was egui's Management (crates/fleet-gui/src/views/management/mod.rs): everything about the fleet, its tabs in groups,
// with the fleet-wide settings egui kept under Settings, General.
export function FleetView({ tab, body }: { tab: FleetTab; body: (tab: FleetTab) => ReactNode }) {
  return (
    <TabbedView className="management" name="Fleet" tabs={FLEET_TABS} groups={FLEET_GROUPS} selected={tab} href={fleetHref} body={body} />
  );
}
