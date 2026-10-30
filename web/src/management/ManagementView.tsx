import type { ReactNode } from "react";
import { MANAGEMENT_TABS, managementHref, navigate, type ManagementTab } from "../shell/routes";
import { Tabs } from "../ui/Tabs";

// A port of crates/fleet-gui/src/views/management/mod.rs: one tab per area of the fleet's configuration.

/** The tab bar and the picked tab's body; `body` draws each tab, so the view owns no tab's state. */
export function ManagementView({ tab, body }: { tab: ManagementTab; body: (tab: ManagementTab) => ReactNode }) {
  return (
    <div className="management">
      <Tabs tabs={MANAGEMENT_TABS} selected={tab} onSelect={(t) => navigate(managementHref(t))} />
      <div className="management-body">{body(tab)}</div>
    </div>
  );
}

/** A tab whose port lands later: it is listed, as egui's tests count every tab, and says so. */
export function NotYet() {
  return (
    <div className="management-page">
      <p className="note">not on the web yet</p>
    </div>
  );
}
