import type { ReactNode } from "react";
import { SETTINGS_TABS, settingsHref, type SettingsTab } from "../shell/routes";
import { TabbedView } from "../shell/TabbedView";

// A port of crates/fleet-gui/src/views/settings/mod.rs, holding only this machine: the app, its daemon, the tools and
// the log, the tab kept while the app runs. The fleet-wide settings are Fleet's.

export function SettingsView({ tab, body }: { tab: SettingsTab; body: (tab: SettingsTab) => ReactNode }) {
  return <TabbedView className="settings" name="Settings" tabs={SETTINGS_TABS} selected={tab} href={settingsHref} body={body} />;
}
