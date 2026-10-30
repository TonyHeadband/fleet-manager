import { useCallback, useEffect, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { GithubAuth } from "../api/generated/GithubAuth";
import type { Persona } from "../api/generated/Persona";

// A port of crates/fleet-gui/src/views/management/personas.rs's PersonasState and of the persona half of
// app/setup.rs: the selected persona, what its fields hold, and the checks and calls made for it.

export interface PersonasState {
  selected: string | null;
  name: string;
  commitName: string;
  commitEmail: string;
  /** The auth type the next GitHub Re-login uses; starts as the persona's own. */
  githubAuth: GithubAuth;
  /** The token typed and not yet sent; emptied as soon as Save token starts the call. */
  pat: string;
  patOpen: boolean;
  patBusy: boolean;
  error: string | null;
  checkError: string | null;
  /** Scripts to run by hand, because no terminal was found to run them in. */
  scripts: { claude?: string; gh?: string };
  /** The nicknames as being edited, by specialist name; blank means none. */
  nicknames: Record<string, string>;
  /** Answers from checks and updates, laid over the app's list until a new one lands; null is deleted here. */
  fresh: Record<string, Persona | null>;
}

export const initialPersonas: PersonasState = {
  selected: null,
  name: "",
  commitName: "",
  commitEmail: "",
  githubAuth: "gh",
  pat: "",
  patOpen: false,
  patBusy: false,
  error: null,
  checkError: null,
  scripts: {},
  nicknames: {},
  fresh: {},
};

/** egui's `select`: the fields take `p`'s values, and what belonged to the persona before goes. */
export const selected = (st: PersonasState, p: Persona): PersonasState => ({
  ...initialPersonas,
  fresh: st.fresh,
  selected: p.id,
  name: p.name,
  commitName: p.commit_name,
  commitEmail: p.commit_email,
  githubAuth: p.github_auth,
  nicknames: { ...p.nicknames },
});

/** Ready with both logins ok: nothing for the periodic check to wait for. */
export const settled = (p: Persona) => p.ready && [p.claude, p.github].every((s) => s?.ok === true);

export const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The tab's state and calls, kept above Management so what was typed outlives the tab, as egui's does. */
export function usePersonasTab(client: FleetClient, base: Persona[] | null, reload: () => void) {
  const [st, setSt] = useState<PersonasState>(initialPersonas);
  const now = useRef(st);
  now.current = st;
  const update = useCallback((f: (s: PersonasState) => PersonasState) => {
    now.current = f(now.current);
    setSt(now.current);
  }, []);

  // A new list from the daemon is newer than any answer laid over the old one.
  useEffect(() => update((s) => ({ ...s, fresh: {} })), [base, update]);

  const listed = base && base.filter((p) => st.fresh[p.id] !== null).map((p) => st.fresh[p.id] ?? p);

  // One check at a time; one asked for meanwhile runs when it lands (egui's `checking` and `check_pending`).
  const checking = useRef(false);
  const queued = useRef<string | null>(null);
  const check = useCallback(
    (id: string) => {
      if (checking.current) {
        queued.current = id;
        return;
      }
      checking.current = true;
      client
        .checkPersona(id)
        .then(
          (p) =>
            update((s) => ({
              ...s,
              fresh: { ...s.fresh, [p.id]: p },
              checkError: s.selected === p.id ? null : s.checkError,
            })),
          // An error is reported only while its persona is still the one shown.
          (e: unknown) => update((s) => (s.selected === id ? { ...s, checkError: `Could not check the logins: ${message(e)}` } : s)),
        )
        .finally(() => {
          checking.current = false;
          const next = queued.current;
          queued.current = null;
          if (next !== null) check(next);
        });
    },
    [client, update],
  );

  /** Selecting a persona checks it. */
  const select = useCallback(
    (p: Persona) => {
      update((s) => selected(s, p));
      check(p.id);
    },
    [update, check],
  );

  return {
    st,
    update,
    listed,
    select,
    check,
    deselect: useCallback(() => update((s) => ({ ...s, selected: null })), [update]),
    remove: (id: string) =>
      client.deletePersona(id).then(
        () => {
          update((s) => ({ ...s, fresh: { ...s.fresh, [id]: null } }));
          reload();
        },
        // egui toasts it and asks for the list again; the web has no toasts, so it shows in the pane.
        (e: unknown) => {
          update((s) => ({ ...s, error: `Could not delete the persona: ${message(e)}` }));
          reload();
        },
      ),
  };
}

export type PersonasApi = ReturnType<typeof usePersonasTab>;
