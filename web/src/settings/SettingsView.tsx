import type { ReactNode } from "react";
import { SETTINGS_TABS, settingsHref, type SettingsTab } from "../shell/routes";
import { TabbedView } from "../shell/TabbedView";

// A port of crates/fleet-gui/src/views/settings/mod.rs: General, Daemon, Extras and Log, the tab kept while the app runs.

export function SettingsView({ tab, body }: { tab: SettingsTab; body: (tab: SettingsTab) => ReactNode }) {
  return <TabbedView className="settings" tabs={SETTINGS_TABS} selected={tab} href={settingsHref} body={body} />;
}
