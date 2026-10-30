import { useEffect, useState, type ReactNode } from "react";
import type { Native, ShortcutsReply, UpdateCheck } from "../native/native";
import type { Notice } from "../shell/messages";
import { MONO_FONTS, UI_FONTS, savePreference, type CloseChoice, type Preferences, type Theme } from "../shell/preferences";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { Check } from "../ui/Check";
import { ErrorLine } from "../ui/ErrorLine";
import { Section } from "../ui/Section";
import { DaemonSections } from "./DaemonSections";
import { SetupWizard } from "./SetupWizard";
import type { DaemonSettingsApi } from "./daemonSettings";
import type { RestartApi } from "./restart";

// A port of crates/fleet-gui/src/views/settings/general.rs's preference sections: Version, Shortcuts, Theme, Font,
// Mono font, Closing the window, Footer, Motion; then the daemon's own sections (DaemonSections).

/** Why a section about the app's own machine is off in a browser (the spec's "disabled with a reason"). */
export const ONLY_IN_APP = "Only in the app";

const THEMES: [Theme, string][] = [
  ["system", "System"],
  ["dark", "Dark"],
  ["light", "Light"],
];

const CLOSE: [CloseChoice, string][] = [
  [null, "Ask"],
  ["keep_running", "Keep daemon running"],
  ["stop_daemon", "Stop daemon"],
];

/** egui's `{what}` in "Could not save {what}: {e}", per preference. */
const WHAT: Record<keyof Preferences, string> = {
  theme: "the theme",
  uiFont: "the font",
  monoFont: "the mono font",
  animations: "the animations setting",
  stickyAlerts: "the footer setting",
  closeChoice: "the close choice",
};

interface Props {
  native: Native | null;
  preferences: Preferences;
  /** Applies the preferences to the window at once; saving them is General's. */
  onPreferences: (next: Preferences) => void;
  update: UpdateCheck;
  /** The app's version in the app, the daemon's in a browser (the page is the daemon's build); "" until known. */
  version: string;
  say: (kind: Notice, text: string) => void;
  daemon: DaemonSettingsApi;
  restart: RestartApi;
  /** The PR styles that load; null until the daemon has listed its definitions. */
  styles: string[] | null;
  onRelaunch: () => void;
}

export function General({ native, preferences, onPreferences, update, version, say, daemon, restart, styles, onRelaunch }: Props) {
  /** egui's `apply_general`: applied at once, then saved; a failed save still applies for this run. A click on the
   * choice already made does nothing. */
  const choose = <K extends keyof Preferences>(key: K, value: Preferences[K]) => {
    if (preferences[key] === value) return;
    onPreferences({ ...preferences, [key]: value });
    savePreference(key, value).then(
      () => say("success", "Settings saved"),
      (e: unknown) => say("error", `Could not save ${WHAT[key]}: ${e instanceof Error ? e.message : String(e)}`),
    );
  };
  const chips = <T,>(key: keyof Preferences, options: [T, ReactNode][], disabled = false) => (
    <div className="chips">
      {options.map(([value, label]) => (
        <Chip
          key={String(value)}
          selected={preferences[key] === value}
          onClick={() => choose(key, value as Preferences[typeof key])}
          disabled={disabled}
          title={disabled ? ONLY_IN_APP : undefined}
        >
          {label}
        </Chip>
      ))}
    </div>
  );
  return (
    <div className="tabbed-page settings-page">
      <Version native={native} update={update} version={version} />
      <Shortcuts native={native} />
      <Section title="Theme">{chips("theme", THEMES)}</Section>
      <Section title="Font">
        {chips(
          "uiFont",
          UI_FONTS.map((f) => [f.id, <span style={{ fontFamily: `"${f.family}"` }}>{f.label}</span>]),
        )}
      </Section>
      <Section title="Mono font">
        {chips(
          "monoFont",
          MONO_FONTS.map((f) => [f.id, <span style={{ fontFamily: `"${f.family}"` }}>{f.label}</span>]),
        )}
      </Section>
      <Section title="Closing the window">{chips("closeChoice", CLOSE, !native)}</Section>
      <Section title="Footer">
        <Check checked={preferences.stickyAlerts} onChange={(on) => choose("stickyAlerts", on)}>
          Keep errors and warnings in the footer until dismissed
        </Check>
      </Section>
      <Section title="Motion">
        <Check
          checked={preferences.animations}
          onChange={(on) => choose("animations", on)}
          title="Card edges, the radar and other motion. Off keeps the window idle."
        >
          Animations
        </Check>
      </Section>
      <DaemonSections native={native} daemon={daemon} restart={restart} styles={styles} />
      <SetupWizard onRelaunch={onRelaunch} />
    </div>
  );
}

/** egui's `version_line`: the running version and what the last new-version check found. */
function Version({ native, update, version }: { native: Native | null; update: UpdateCheck; version: string }) {
  let line: ReactNode = `Version v${version}`;
  if (update.kind === "current") line = `Version v${version} — up to date`;
  else if (update.kind === "failed") line = "Could not check for updates";
  else if (update.kind === "available")
    line = (
      <>
        <span>{`v${update.version} available`}</span>
        <span>—</span>
        <a
          href={update.page}
          onClick={(e) => {
            e.preventDefault();
            void native?.openUrl(update.page);
          }}
        >
          Download
        </a>
      </>
    );
  return (
    <Section title="Version">
      <p className="settings-line">{line}</p>
    </Section>
  );
}

const describe = (r: ShortcutsReply) => {
  if (r.state === "none") return "No shortcuts yet";
  if (r.state === "stale") return "Shortcuts point to an old location —";
  const places = [r.menu && "Start menu", r.desktop && "Desktop"].filter(Boolean);
  return `Shortcuts in: ${places.join(", ")}`;
};

/** egui's `shortcuts_row`: this app's Start menu and Desktop entries. In a browser they show disabled with why. */
function Shortcuts({ native }: { native: Native | null }) {
  const [state, setState] = useState<ShortcutsReply | null>(null);
  const [pick, setPick] = useState({ menu: true, desktop: false });
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    native?.shortcuts().then(setState, () => {});
  }, [native]);

  const change = (action: "add" | "remove" | "update") => {
    if (!native) return;
    // Update rewrites the entries that exist, whatever is ticked.
    const which = action === "update" && state ? { menu: state.menu, desktop: state.desktop } : pick;
    native.changeShortcuts(action, which.menu, which.desktop).then(
      (r) => {
        setState(r);
        setError(null);
      },
      (e: unknown) => setError(`Could not change the shortcuts: ${e instanceof Error ? e.message : String(e)}`),
    );
  };
  const off = !native || !(pick.menu || pick.desktop);
  const title = native ? undefined : ONLY_IN_APP;
  return (
    <Section title="Shortcuts">
      {state && (
        <div className="settings-row">
          <span className="settings-line">{describe(state)}</span>
          {state.state === "stale" && (
            <Button variant="primary" onClick={() => change("update")}>
              Update
            </Button>
          )}
        </div>
      )}
      <div className="settings-row">
        <Check checked={pick.menu} disabled={!native} title={title} onChange={(menu) => setPick({ ...pick, menu })}>
          Start menu
        </Check>
        <Check checked={pick.desktop} disabled={!native} title={title} onChange={(desktop) => setPick({ ...pick, desktop })}>
          Desktop
        </Check>
      </div>
      <div className="settings-row">
        <Button disabled={off} title={title} onClick={() => change("add")}>
          Add shortcuts
        </Button>
        <Button disabled={off} title={title} onClick={() => change("remove")}>
          Remove shortcuts
        </Button>
      </div>
      {error !== null && <ErrorLine text={error} />}
    </Section>
  );
}
