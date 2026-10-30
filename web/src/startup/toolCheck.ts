import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { ToolId } from "../api/generated/ToolId";
import type { ToolList } from "../api/generated/ToolList";
import type { Native } from "../native/native";

// A port of crates/fleet-gui/src/tool_check.rs: the daemon's answer about the tools, a check or an install in flight,
// and why the list is not the latest answer. The first run's Requirements, the relaunched wizard and Extras share it.

const EMPTY: ToolList = { tools: [], checked_at: 0n, path: null };

interface Options {
  /** The app may run an install itself: it is the app, and the daemon is on this machine. */
  canInstall: boolean;
  /** The daemon is on this machine: each answer's PATH is the app's too (egui `adopt_path`). */
  local: boolean;
  /** An answer already in hand (the first run's); without it the list starts empty and nothing is asked yet. */
  first?: { list: ToolList; note: string | null };
}

export function useToolCheck(client: FleetClient, native: Native | null, { canInstall, local, first }: Options) {
  const [list, setList] = useState(first?.list ?? EMPTY);
  const [note, setNote] = useState(first?.note ?? null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState<ToolId | null>(null);
  // A second Check again starts a fresh check; the answer to the first is no longer waited for.
  const asked = useRef(0);
  const now = useRef({ client, native, local });
  now.current = { client, native, local };

  const apply = useCallback((l: ToolList) => {
    const { native, local } = now.current;
    setList(l);
    setNote(null);
    if (native && local && l.path) void native.adoptPath();
  }, []);
  const fail = useCallback((e: unknown) => setNote(e instanceof Error ? e.message : String(e)), []);

  const recheck = useCallback(() => {
    const n = ++asked.current;
    const current = () => n === asked.current;
    setChecking(true);
    void now.current.client
      .recheckTools()
      .then(
        (l) => current() && apply(l),
        (e: unknown) => current() && fail(e),
      )
      .finally(() => current() && setChecking(false));
  }, [apply, fail]);

  const install = (tool: ToolId) => {
    if (!native) return;
    setInstalling(tool);
    // When the window closes, whatever happened in it, the daemon says what is there now.
    void native
      .installTool(tool)
      .then(() => client.recheckTools())
      .then(apply, fail)
      .finally(() => setInstalling(null));
  };

  return {
    list,
    note,
    checking,
    installing,
    /** Nothing in flight: an Install click would otherwise race the answer. */
    canInstall: canInstall && installing === null && !checking,
    recheck,
    install,
  };
}

export type ToolCheck = ReturnType<typeof useToolCheck>;
