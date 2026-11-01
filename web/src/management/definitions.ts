import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { DefFile } from "../api/generated/DefFile";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import type { Notice } from "../shell/messages";
import type { FleetTab } from "../shell/routes";

// A port of crates/fleet-gui/src/views/management/definitions.rs's DefsState and of the definition calls in
// app/fleet.rs: every orchestrator, specialist, skill and PR style, each opened file with its own draft and error.

/** `{persona}/{kind}/{name}`: one definition. A PR style belongs to no persona, so its persona is empty. */
export type DefKey = string;

/** The Fleet tabs that each list one kind. */
export const DEF_TABS = {
  orchestrators: "orchestrator",
  specialists: "specialist",
  skills: "skill",
  "pr-styles": "pr_style",
} as const satisfies Partial<Record<FleetTab, DefKind>>;

export const defKey = (persona: string, kind: DefKind, name: string): DefKey => `${persona}/${kind}/${name}`;

/** egui's `usable`: the names of `kind` that load. */
export const usable = (defs: DefInfo[], kind: DefKind) => defs.filter((d) => d.kind === kind && !d.error).map((d) => d.name);

export interface DefsState {
  /** The active persona's orchestrators, specialists and skills; null until the daemon first answers for it. */
  list: DefInfo[] | null;
  /** The fleet-wide PR styles; null until they first load. */
  styles: DefInfo[] | null;
  selected: { kind: DefKind; name: string } | null;
  /** Each opened definition's file as the daemon last sent it. */
  files: Record<DefKey, DefFile>;
  /** Editable text: the user's file being edited, or a built-in being overridden. A built-in without one is read-only. */
  drafts: Record<DefKey, string>;
  /** Why the last read, save or reset of a definition failed. */
  errors: Record<DefKey, string>;
  /** The save in flight and the text it sent. */
  saving: { key: DefKey; text: string } | null;
}

export const initialDefs: DefsState = { list: null, styles: null, selected: null, files: {}, drafts: {}, errors: {}, saving: null };

type Say = (notice: Notice, text: string) => void;

/** What the hook keeps: a persona's list and selection are held apart, and `st` shows the active persona's. */
interface Held extends DefsState {
  lists: Record<string, DefInfo[]>;
  picks: Record<string, NonNullable<DefsState["selected"]>>;
}

const without = <T>(r: Record<DefKey, T>, key: DefKey) => {
  const { [key]: _, ...rest } = r;
  return rest;
};

/** The daemon's file arrived. A draft being edited stays; a user file gets one to edit. */
export function loaded(st: DefsState, key: DefKey, file: DefFile): DefsState {
  return {
    ...st,
    files: { ...st.files, [key]: file },
    drafts: file.builtin || key in st.drafts ? st.drafts : { ...st.drafts, [key]: file.text },
    errors: without(st.errors, key),
  };
}

/** A save landed: what it sent is now the user's file. */
export function saved(st: DefsState): DefsState {
  if (!st.saving) return st;
  const { key, text } = st.saving;
  const file = st.files[key];
  return {
    ...st,
    saving: null,
    files: file ? { ...st.files, [key]: { ...file, text, builtin: false } } : st.files,
    errors: without(st.errors, key),
  };
}

/** The user's copy is gone: the built-in is read again, with no draft over it. */
export function reset(st: DefsState, key: DefKey): DefsState {
  return { ...st, drafts: without(st.drafts, key), files: without(st.files, key), errors: without(st.errors, key) };
}

/** An override not saved yet is a change in itself: Save writes it, Revert drops it. */
export const defDirty = (draft: string, file: DefFile) => draft !== file.text || file.builtin;

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The definitions and their calls, kept above Management so drafts outlive the tabs, the view and a persona switch. */
export function useDefinitions(client: FleetClient, persona: string | null, say: Say, onNicknameChanged?: () => void) {
  const [held, setHeld] = useState<Held>({ ...initialDefs, lists: {}, picks: {} });
  // Read synchronously, so a second save asked for before a render is refused, as egui's is.
  const now = useRef(held);
  now.current = held;
  const update = useCallback((f: (s: Held) => Held) => {
    now.current = f(now.current);
    setHeld(now.current);
  }, []);
  const active = persona ?? "";
  const st: DefsState = { ...held, list: held.lists[active] ?? null, selected: held.picks[active] ?? null };

  // Lists asked so far: one answered after a newer one is stale (entering a tab, a persona switch and a save's refresh).
  // Counted per persona: another persona's answer never makes this one's stale.
  const lists = useRef<Record<string, number>>({});
  const loadList = useCallback(() => {
    if (persona === null) return;
    const seq = (lists.current[persona] = (lists.current[persona] ?? 0) + 1);
    const latest = () => seq === lists.current[persona];
    client.personaDefinitions(persona).then(
      (list) => latest() && update((s) => ({ ...s, lists: { ...s.lists, [persona]: list } })),
      // egui keeps what it had and toasts; a tab with no list yet stays on Loading….
      (e: unknown) => latest() && say("error", `Could not load the orchestrators and specialists: ${message(e)}`),
    );
  }, [client, persona, update, say]);

  const styleLists = useRef(0);
  const loadStyles = useCallback(() => {
    const seq = ++styleLists.current;
    client.definitions().then(
      (styles) => seq === styleLists.current && update((s) => ({ ...s, styles })),
      (e: unknown) => seq === styleLists.current && say("error", `Could not load the PR styles: ${message(e)}`),
    );
  }, [client, update, say]);

  // Per key, bumped as a save goes out and as it lands: a read sent before either may answer with the file the save
  // replaced, so its answer is dropped.
  const saves = useRef<Record<DefKey, number>>({});
  const bump = (key: DefKey) => (saves.current[key] = (saves.current[key] ?? 0) + 1);

  const fail = (key: DefKey) => (e: unknown) => update((s) => ({ ...s, errors: { ...s.errors, [key]: message(e) } }));

  // A PR style is the fleet's, every other kind the persona's.
  const keyOf = (kind: DefKind, name: string) => defKey(kind === "pr_style" ? "" : active, kind, name);
  const read = (kind: DefKind, name: string) =>
    kind === "pr_style" || persona === null ? client.definition(kind, name) : client.personaDefinition(persona, kind, name);

  const open = (kind: DefKind, name: string) => {
    const key = keyOf(kind, name);
    const asked = saves.current[key];
    const fresh = () => saves.current[key] === asked;
    // A new definition not saved yet (a draft with no file) is dropped by opening anything else.
    update((s) => ({
      ...s,
      drafts: Object.fromEntries(Object.entries(s.drafts).filter(([k]) => k in s.files || k === key)),
      picks: { ...s.picks, [active]: { kind, name } },
    }));
    read(kind, name).then(
      (file) => fresh() && update((s) => ({ ...s, ...loaded(s, key, file) })),
      (e: unknown) => fresh() && fail(key)(e),
    );
  };

  const reload = (kind: DefKind) => (kind === "pr_style" ? loadStyles() : loadList());

  return {
    st,
    persona: active,
    loadList,
    loadStyles,
    open,
    key: keyOf,
    edit: (key: DefKey, text: string) => update((s) => ({ ...s, drafts: { ...s.drafts, [key]: text } })),
    override: (key: DefKey) => {
      const file = now.current.files[key];
      if (file) update((s) => ({ ...s, drafts: { ...s.drafts, [key]: file.text } }));
    },
    /** An unsaved draft with no file: the first Save writes it, and until then the list does not show it. */
    create: (kind: DefKind, name: string, text: string) => {
      const key = keyOf(kind, name);
      update((s) => ({
        ...s,
        drafts: { ...s.drafts, [key]: text },
        errors: without(s.errors, key),
        picks: { ...s.picks, [active]: { kind, name } },
      }));
    },
    /** Resolves once the file is written; rejects (the message is also under the definition) when it is not. */
    save: (kind: DefKind, name: string): Promise<void> => {
      const key = keyOf(kind, name);
      const text = now.current.drafts[key];
      if (now.current.saving || text === undefined) return Promise.reject(new Error("Nothing to save, or a save is under way"));
      bump(key);
      update((s) => ({ ...s, saving: { key, text }, errors: without(s.errors, key) }));
      const put = kind === "pr_style" || persona === null ? client.saveDefinition(kind, name, text) : client.savePersonaDefinition(persona, kind, name, text);
      return put.then(
        () => {
          bump(key);
          // A new definition has no file yet: what it sent is the file now.
          update((s) => {
            const n = saved(s);
            return { ...s, ...n, files: key in n.files ? n.files : { ...n.files, [key]: { kind, name, text, builtin: false } } };
          });
          say("success", `${name} saved`);
          reload(kind);
        },
        // The message goes under the editor; the draft stays to be fixed.
        (e: unknown) => {
          update((s) => ({ ...s, saving: null, errors: { ...s.errors, [key]: message(e) } }));
          throw e;
        },
      );
    },
    /** An empty nickname clears it. The list is read again so the row shows it, and `onNicknameChanged` lets the app's
     * persona list (which labels the board and drawer) follow; a refusal is for the caller to show. */
    setNickname: async (name: string, nickname: string) => {
      if (persona === null) throw new Error("No persona is active");
      await client.setNickname(persona, name, nickname);
      loadList();
      onNicknameChanged?.();
    },
    /** Moves the persona's file to the other role, then reopens it there; a refusal is for the caller to show. */
    move: async (kind: DefKind, name: string, to: DefKind) => {
      if (persona === null) throw new Error("No persona is active");
      await client.movePersonaDefinition(persona, kind, name, to);
      const key = keyOf(kind, name);
      update((s) => ({ ...s, ...reset(s, key) }));
      loadList();
      open(to, name);
    },
    revert: (key: DefKey) =>
      update((s) => {
        const file = s.files[key];
        // A new definition has no file to go back to: Revert discards it.
        if (!file) {
          const picked = s.picks[active];
          const gone = picked !== undefined && keyOf(picked.kind, picked.name) === key;
          return { ...s, drafts: without(s.drafts, key), errors: without(s.errors, key), picks: gone ? without(s.picks, active) : s.picks };
        }
        const drafts = file.builtin ? without(s.drafts, key) : { ...s.drafts, [key]: file.text };
        return { ...s, drafts, errors: without(s.errors, key) };
      }),
    /** Asked after the confirm: deletes the user's copy, then reads the built-in again if it is still open. */
    reset: (kind: DefKind, name: string) => {
      const key = keyOf(kind, name);
      const del = kind === "pr_style" || persona === null ? client.resetDefinition(kind, name) : client.resetPersonaDefinition(persona, kind, name);
      del.then(() => {
        update((s) => ({ ...s, ...reset(s, key) }));
        say("success", `${name} reset to built-in`);
        const sel = now.current.picks[active];
        if (sel?.kind === kind && sel.name === name) open(kind, name);
        reload(kind);
      }, fail(key));
    },
  };
}

export type DefinitionsApi = ReturnType<typeof useDefinitions>;
