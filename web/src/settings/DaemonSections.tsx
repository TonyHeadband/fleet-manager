import type { ReactNode } from "react";
import type { Effort } from "../api/generated/Effort";
import type { PermissionMode } from "../api/generated/PermissionMode";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";
import { Check } from "../ui/Check";
import { Section } from "../ui/Section";
import { changeOf, pendingFolders, saveBlockers, type DaemonSettingsApi, type SaveFor, type Typed } from "./daemonSettings";
import { ONLY_IN_APP } from "./General";
import type { RestartApi } from "./restart";

// A port of crates/fleet-gui/src/views/settings/general.rs's daemon sections: Tasks folder, Personas folder, Limits,
// Pull requests and Agents. Each Save writes config.toml and restarts the daemon, through the one restart.

const MODES: PermissionMode[] = ["auto", "acceptEdits", "manual", "dontAsk"];
const EFFORTS: Effort[] = ["low", "medium", "high", "xhigh", "max"];
const GATE_BUILDS = ["shared", "clean"] as const;

/** egui's `LIMITS`: the label, and what the limit does, which is also the field's hover. */
const LIMITS: [string, string][] = [
  ["Max breakdown versions", "Most breakdown versions one task may accumulate. A new breakdown after that is refused."],
  ["Max steps", "Most steps one breakdown may declare."],
  ["Max parallel specialists", "Most specialists one orchestrator may run at once. Starting another is refused."],
  ["Max PR attempts", "Most times one task may try to open its pull request. A refused attempt after that is final."],
  ["Max concurrent tasks", "Most orchestrated tasks one product may have working at once. Starting another is refused. Empty: no cap."],
  [
    "Max active products",
    "Most products that may have orchestrated tasks working at once. Starting a task in another is refused. Empty: no cap.",
  ],
];

/** Each section's heading, its Save's name when "Save" alone is ambiguous, and its note. */
const SECTIONS: Record<SaveFor, { title: string; save: string; note: string }> = {
  tasksRoot: {
    title: "Tasks folder",
    save: "Save",
    note: "Applies to new tasks only. Saving restarts the daemon; running agents keep going.",
  },
  personasDir: {
    title: "Personas folder",
    save: "Save personas folder",
    note: "Saving restarts the daemon. Existing personas' folders, with their Claude and GitHub logins, are not moved: move them by hand or log the personas in again.",
  },
  limits: {
    title: "Limits",
    save: "Save limits",
    note: "Saving restarts the daemon; running agents keep going. A product can override each limit except Max active products on Fleet, Products.",
  },
  pr: {
    title: "Pull requests",
    save: "Save pull requests",
    note: "Saving restarts the daemon; running agents keep going. A product can override each of these on Fleet, Products.",
  },
  agentDefaults: {
    title: "Agents",
    save: "Save agent defaults",
    note: "Used by every agent whose definition and product name neither. Saving restarts the daemon; running agents keep going.",
  },
};

interface Props {
  native: Native | null;
  daemon: DaemonSettingsApi;
  restart: RestartApi;
  /** The sections to draw, in this order: the folders are Settings, Daemon's; the rest are Fleet's rules. */
  kinds: SaveFor[];
  /** The PR styles that load, for the Pull requests section; null until the daemon has listed its definitions. */
  styles?: string[] | null;
}

export function DaemonSections({ native, daemon, restart, kinds, styles = null }: Props) {
  // In a browser the sections write a config on another machine: each says so, its Save off.
  if (!native) return kinds.map((kind) => <Saving key={kind} kind={kind} blocked={ONLY_IN_APP} note={ONLY_IN_APP} />);
  const { saved, typed, set } = daemon;
  if (!saved || !typed) return null;
  const blockers = saveBlockers(saved, typed, restart.blocker);
  const pending = pendingFolders(saved);
  const save = (kind: SaveFor) => {
    const change = changeOf(kind, typed);
    if (change) restart.ask(change);
  };
  /** A section's Save and note: the note gives way to the restart this section asked for, or Restart daemon. */
  const saving = (kind: SaveFor, children: ReactNode, saveInRow = false) => (
    <Saving
      kind={kind}
      blocked={blockers[kind]}
      note={restart.restarting === kind || restart.restarting === "restart" ? "Restarting the daemon…" : SECTIONS[kind].note}
      onSave={() => save(kind)}
      saveInRow={saveInRow}
    >
      {children}
    </Saving>
  );
  const folder = (kind: "tasksRoot" | "personasDir") =>
    saving(
      kind,
      <>
        <input
          className="mono"
          aria-label={SECTIONS[kind].title}
          value={typed[kind]}
          onChange={(e) => set({ [kind]: e.target.value })}
          spellCheck={false}
        />
        <Button
          onClick={() =>
            void native.pickFolder(typed[kind]).then(
              (picked) => picked !== null && set({ [kind]: picked }),
              () => {},
            )
          }
        >
          Browse…
        </Button>
      </>,
      true,
    );
  const pr = (patch: Partial<Typed["pr"]>) => set({ pr: { ...typed.pr, ...patch } });
  // Without the definitions list, the current style is all there is to show; an unlisted one is shown as it is.
  const shownStyles = !styles ? [typed.pr.style] : styles.includes(typed.pr.style) ? styles : [typed.pr.style, ...styles];
  const has = (kind: SaveFor) => kinds.includes(kind);
  return (
    <>
      {has("tasksRoot") && folder("tasksRoot")}
      {has("personasDir") && folder("personasDir")}
      {has("tasksRoot") && pending !== null && (
        // egui's `pending_restart`: what the relaunched wizard wrote, waiting for the daemon's restart.
        <div className="settings-pending">
          <p className="settings-line">Saved: {pending} — takes effect when the background service restarts.</p>
          <div className="settings-row">
            <Button
              variant="primary"
              disabled={restart.blocker !== null}
              title={restart.blocker ?? undefined}
              onClick={() => restart.ask({ kind: "restart" })}
            >
              Restart now
            </Button>
            {restart.restarting !== null && <span className="note">Restarting the daemon…</span>}
          </div>
        </div>
      )}
      {has("limits") &&
        saving(
          "limits",
          LIMITS.map(([label, hint], i) => (
            <label key={label} className="settings-row settings-limit">
              <span>{label}</span>
              <input
                value={typed.limits[i]}
                title={hint}
                onChange={(e) => set({ limits: typed.limits.map((t, j) => (j === i ? e.target.value : t)) })}
              />
            </label>
          )),
        )}
      {has("pr") &&
        saving(
          "pr",
          <>
            <label className="settings-row">
              <span>Style</span>
              <select aria-label="Pull request style" value={typed.pr.style} onChange={(e) => pr({ style: e.target.value })}>
                {shownStyles.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
            </label>
            <label className="settings-row settings-pr">
              <span>Attribution</span>
              <input
                value={typed.pr.attribution}
                title="A line added to the end of every pull request body. Empty adds nothing."
                onChange={(e) => pr({ attribution: e.target.value })}
              />
            </label>
            <Check checked={typed.pr.draft} onChange={(draft) => pr({ draft })}>
              Draft pull requests
            </Check>
            <label className="settings-row settings-pr">
              <span>PR labels</span>
              <input
                value={typed.pr.labels}
                title="Comma-separated labels put on every pull request."
                onChange={(e) => pr({ labels: e.target.value })}
              />
            </label>
            <label className="settings-row">
              <span>Gate build</span>
              <select
                value={typed.pr.gateBuild}
                title="shared: the gate builds in the task's folder, reusing what the agents built. clean: it builds in a fresh folder that is removed after."
                onChange={(e) => pr({ gateBuild: e.target.value as Typed["pr"]["gateBuild"] })}
              >
                {GATE_BUILDS.map((g) => (
                  <option key={g}>{g}</option>
                ))}
              </select>
            </label>
          </>,
        )}
      {has("agentDefaults") &&
        saving(
          "agentDefaults",
          <>
            <label className="settings-row">
              <span>Default mode</span>
              <select value={typed.mode} onChange={(e) => set({ mode: e.target.value as PermissionMode })}>
                {MODES.map((m) => (
                  <option key={m}>{m}</option>
                ))}
              </select>
            </label>
            <label className="settings-row">
              <span>Default effort</span>
              <select value={typed.effort} onChange={(e) => set({ effort: e.target.value as Effort })}>
                {EFFORTS.map((m) => (
                  <option key={m}>{m}</option>
                ))}
              </select>
            </label>
          </>,
        )}
    </>
  );
}

/** A daemon section: its rows, Save (on the folder's row, else under the rows) with its blocker as the title, and the
 * note under it all. */
function Saving(props: {
  kind: SaveFor;
  blocked: string | null;
  note: string;
  onSave?: () => void;
  saveInRow?: boolean;
  children?: ReactNode;
}) {
  const { kind, blocked, note, onSave, saveInRow = false, children } = props;
  const { title, save: name } = SECTIONS[kind];
  const button = (
    <Button
      variant="primary"
      aria-label={name === "Save" ? undefined : name}
      disabled={blocked !== null}
      title={blocked ?? undefined}
      onClick={onSave}
    >
      Save
    </Button>
  );
  return (
    <Section title={title}>
      {saveInRow ? (
        <div className="settings-row settings-folder">
          {children}
          {button}
        </div>
      ) : (
        <>
          {children}
          {button}
        </>
      )}
      <p className="note">{note}</p>
    </Section>
  );
}
