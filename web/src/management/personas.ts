import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { GithubAuth } from "../api/generated/GithubAuth";
import type { Persona } from "../api/generated/Persona";
import type { PersonaUpdate } from "../api/generated/PersonaUpdate";
import { askOnce, claudeCommand, claudeLoginScript, ghLoginScript, ghProgram } from "../api/login";
import type { Native } from "../native/native";
import type { Notice } from "../shell/messages";

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

type Which = "claude" | "gh";

const NO_CHANGE: PersonaUpdate = { name: null, commit_name: null, commit_email: null, github_auth: null, nicknames: null };

/** egui's `cleaned`: the nicknames as Save sends them, trimmed, blank ones left out. */
export const cleaned = (nicknames: Record<string, string>) =>
  Object.fromEntries(
    Object.entries(nicknames)
      .map(([k, v]) => [k, v.trim()])
      .filter(([, v]) => v !== ""),
  );

export const sameNicknames = (a: Record<string, string>, b: Record<string, string>) =>
  Object.keys(a).length === Object.keys(b).length && Object.entries(a).every(([k, v]) => b[k] === v);

/** The tab's state and calls, kept above Management so what was typed outlives the tab, as egui's does. */
/**
 * `native` and `local`: the app on the daemon's machine opens login terminals and hands tokens to gh itself; anywhere
 * else the login script is shown to run by hand, and the token waits for the app.
 */
export function usePersonasTab(
  client: FleetClient,
  base: Persona[] | null,
  reload: () => void,
  native: Native | null,
  local: boolean,
  say: (notice: Notice, text: string) => void,
) {
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
              // A check that lands after a delete does not bring the row back.
              fresh: s.fresh[p.id] === null ? s.fresh : { ...s.fresh, [p.id]: p },
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

  // The persona list's commands and tasks root, for the scripts a browser shows; asked once, as the wizard does.
  const meta = useMemo(() => askOnce(client.personas), [client]);
  const loadMeta = useCallback(() => void meta().catch(() => {}), [meta]);

  /** Applies `f` only while `id` is still the persona shown: an answer for one no longer selected is dropped. */
  const forShown = (id: string, f: (s: PersonasState) => PersonasState) => update((s) => (s.selected === id ? f(s) : s));

  const open = (p: Persona, which: Which) => {
    update((s) => ({ ...s, error: null, scripts: { ...s.scripts, [which]: undefined } }));
    const byHand = (script: string) => forShown(p.id, (s) => ({ ...s, scripts: { ...s.scripts, [which]: script } }));
    if (native && local) {
      native.runLogin(p.id, which).then(
        (r) => r.script !== null && byHand(r.script),
        (e: unknown) => forShown(p.id, (s) => ({ ...s, error: message(e) })),
      );
    } else {
      meta().then(
        (m) => byHand(which === "claude" ? claudeLoginScript(p, claudeCommand(m), m.tasks_root) : ghLoginScript(p, ghProgram(m))),
        (e: unknown) => forShown(p.id, (s) => ({ ...s, error: `Could not get the login commands: ${message(e)}` })),
      );
    }
  };

  /** Re-login to GitHub with the persona's auth type as it now is: a terminal for gh, the token field for a PAT. */
  const beginGithub = (p: Persona) => {
    if (p.github_auth === "gh") open(p, "gh");
    else update((s) => ({ ...s, pat: "", patOpen: true, error: null }));
  };

  const save = (p: Persona, change: Partial<PersonaUpdate>, relogin = false) => {
    update((s) => ({ ...s, error: null }));
    client.updatePersona(p.id, { ...NO_CHANGE, ...change }).then(
      (saved) => {
        update((s) => ({ ...s, fresh: { ...s.fresh, [saved.id]: saved } }));
        // The board, the drawer and the breakdowns read the app's list: a new name or nickname shows there too.
        reload();
        forShown(saved.id, (s) => ({ ...s, error: null, githubAuth: saved.github_auth, nicknames: { ...saved.nicknames } }));
        if (relogin && now.current.selected === saved.id) beginGithub(saved);
      },
      (e: unknown) => update((s) => ({ ...s, error: `Could not save: ${message(e)}` })),
    );
  };

  return {
    st,
    update,
    loadMeta,
    rename: (p: Persona) => save(p, { name: now.current.name.trim() }),
    saveIdentity: (p: Persona) => save(p, { commit_name: now.current.commitName.trim(), commit_email: now.current.commitEmail.trim() }),
    saveNicknames: (p: Persona) => save(p, { nicknames: cleaned(now.current.nicknames) }),
    relogin: (p: Persona, which: Which) => {
      if (which === "claude") open(p, "claude");
      // A changed auth choice is saved first; the login starts when the daemon has it.
      else if (now.current.githubAuth !== p.github_auth) save(p, { github_auth: now.current.githubAuth }, true);
      else beginGithub(p);
    },
    /** Hands the token to gh; it leaves the field (and this state) as the call starts. */
    submitPat: (p: Persona) => {
      if (!native) return;
      const token = now.current.pat;
      update((s) => ({ ...s, pat: "", patBusy: true, error: null }));
      native.savePat(p.id, token).then(
        () => {
          forShown(p.id, (s) => ({ ...s, patBusy: false, patOpen: false, error: null }));
          if (now.current.selected === p.id) check(p.id);
        },
        (e: unknown) => forShown(p.id, (s) => ({ ...s, patBusy: false, error: message(e) })),
      );
    },
    canSavePat: native !== null && local,
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
        // egui toasts it and asks for the list again: the daemon's says what is left.
        (e: unknown) => {
          say("error", `Could not delete the persona: ${message(e)}`);
          reload();
        },
      ),
  };
}

export type PersonasApi = ReturnType<typeof usePersonasTab>;
