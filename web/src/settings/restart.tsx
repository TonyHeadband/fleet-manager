import { useCallback, useRef, useState, type ReactNode } from "react";
import type { Agent } from "../api/generated/Agent";
import type { Native, RestartReply, SettingsChange } from "../native/native";
import { dialogText, summarize } from "../shell/close";
import type { Notice } from "../shell/messages";
import { ConfirmDialog } from "../ui/ConfirmDialog";

// A port of egui's restart (crates/fleet-gui/src/app.rs: ask_restart, start_restart, restart_blocker, the
// "Restart the daemon?" dialog and the Restarted result): one path for Settings' Saves, Restart daemon and the
// version offer's Restart.

/** egui's `RestartFor`: which Save, or Restart daemon itself. */
export type RestartFor = SettingsChange["kind"];

/** Why Save and Restart daemon cannot run now, in egui's order: a restart, then a Queued agent (a restart cuts its
 * spawn short), then a task being started, then a repo request being answered. */
export function restartBlocker(o: { restarting: boolean; agents: readonly Agent[]; spawning: boolean; deciding: boolean }) {
  if (o.restarting) return "The daemon is restarting";
  if (o.agents.some((a) => a.status === "queued")) return "An agent is still starting; wait until it runs";
  if (o.spawning) return "A task is being started";
  if (o.deciding) return "A repo request is being answered";
  return null;
}

const REQUEST_ENDS = {
  save: " An agent is waiting for an answer about a repo. Saving restarts the daemon, which ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
  restart:
    " An agent is waiting for an answer about a repo. Restarting the daemon ends that request: it is marked interrupted by daemon restart, and the agent has to ask again.",
};

/** The dialog's body: the close dialog's words, then what happens to a pending repo request. */
export function confirmText(agents: readonly Agent[], kind: RestartFor): string {
  const asking = agents.some((a) => a.repo_request);
  return dialogText(summarize([...agents])) + (asking ? REQUEST_ENDS[kind === "restart" ? "restart" : "save"] : "");
}

/** What a Save says when the daemon had no pidfile, so it still runs and the change waits (egui's `AlreadyRunning`). */
const SAVED_ONLY: Record<Exclude<RestartFor, "restart">, string> = {
  tasksRoot: "Saved. Restart the daemon to use the new folder.",
  personasDir: "Saved. Restart the daemon to use the new folder.",
  limits: "Saved. Restart the daemon to use the new limits.",
  pr: "Saved. Restart the daemon to use the new pull request settings.",
  agentDefaults: "Saved. Restart the daemon to use the new defaults.",
};

const NO_PIDFILE = "Could not restart the daemon: no daemon.pid: this daemon was not started by a version that writes one";

interface Options {
  /** The stream's first snapshot came: without it nobody knows what runs, so a restart asks. */
  loaded: boolean;
  agents: readonly Agent[];
  spawning: boolean;
  deciding: boolean;
  say: (kind: Notice, text: string) => void;
  /** A daemon was started: the page follows it. */
  onRestarted: (reply: RestartReply) => void;
  /** The call settled: `ok` when the config was written and nothing failed after. */
  onSettled?: (change: SettingsChange, ok: boolean) => void;
}

export function useRestart(native: Native | null, { loaded, agents, spawning, deciding, say, onRestarted, onSettled }: Options) {
  const [restarting, setRestarting] = useState<RestartFor | null>(null);
  const [asking, setAsking] = useState<SettingsChange | null>(null);
  // Read synchronously, so a second click before the re-render starts nothing.
  const busy = useRef(false);
  const blockerNow = () => restartBlocker({ restarting: busy.current, agents, spawning, deciding });
  const blocker = restartBlocker({ restarting: restarting !== null, agents, spawning, deciding });

  const start = (change: SettingsChange) => {
    if (!native || busy.current) return;
    busy.current = true;
    setRestarting(change.kind);
    native.saveAndRestart(change).then(
      (reply) => {
        end();
        onSettled?.(change, true);
        if (reply.outcome === "started") {
          say("success", "Daemon restarted");
          onRestarted(reply);
        } else if (change.kind === "restart") say("error", NO_PIDFILE);
        else say("info", SAVED_ONLY[change.kind]);
      },
      (e: unknown) => {
        // The config stays written when only the restart failed; Restart daemon can try again.
        end();
        onSettled?.(change, false);
        say("error", e instanceof Error ? e.message : String(e));
      },
    );
  };
  const end = () => {
    busy.current = false;
    setRestarting(null);
  };

  /** egui's `ask_restart`: asks first while agents run or before the first snapshot, else starts. */
  const ask = (change: SettingsChange) => {
    if (!native || blockerNow() !== null) return;
    if (!loaded || agents.some((a) => a.status !== "stopped")) setAsking(change);
    else start(change);
  };

  const cancel = useCallback(() => setAsking(null), []);
  const dialog: ReactNode = asking && (
    <ConfirmDialog
      title="Restart the daemon?"
      body={confirmText(agents, asking.kind)}
      go="Restart anyway"
      goVariant="default"
      cancelVariant="primary"
      goFirst
      onGo={() => {
        setAsking(null);
        // An agent queued meanwhile cancels it without a word, as egui's.
        if (blockerNow() === null) start(asking);
      }}
      onCancel={cancel}
    />
  );

  return {
    restarting,
    blocker,
    /** egui's `daemon_blocker`: calls a restart would cut short (New task, Grant, Refuse) wait for it. */
    daemonBlocker: restarting !== null ? "The daemon is restarting" : null,
    ask,
    dialog,
  };
}

export type RestartApi = ReturnType<typeof useRestart>;
