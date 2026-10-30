import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { DefFile } from "../api/generated/DefFile";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import type { Notice } from "../shell/messages";
import type { ManagementTab } from "../shell/routes";

// A port of crates/fleet-gui/src/views/management/definitions.rs's DefsState and of the definition calls in
// app/fleet.rs: every orchestrator, specialist, skill and PR style, each opened file with its own draft and error.

/** `{kind}/{name}`: one definition. */
export type DefKey = string;

/** The Management tabs that each list one kind. */
export const DEF_TABS = {
  orchestrators: "orchestrator",
  specialists: "specialist",
  skills: "skill",
  "pr-styles": "pr_style",
} as const satisfies Partial<Record<ManagementTab, DefKind>>;

export const defKey = (kind: DefKind, name: string): DefKey => `${kind}/${name}`;

/** egui's `usable`: the names of `kind` that load. */
export const usable = (defs: DefInfo[], kind: DefKind) => defs.filter((d) => d.kind === kind && !d.error).map((d) => d.name);

export interface DefsState {
  /** The daemon's list; null until it first answers. */
  list: DefInfo[] | null;
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

export const initialDefs: DefsState = { list: null, selected: null, files: {}, drafts: {}, errors: {}, saving: null };

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

/** The definitions and their calls, kept above Management so drafts outlive the tabs and the view. */
export function useDefinitions(client: FleetClient, say: (notice: Notice, text: string) => void) {
  const [st, setSt] = useState<DefsState>(initialDefs);
  // Read synchronously, so a second save asked for before a render is refused, as egui's is.
  const now = useRef(st);
  now.current = st;
  const update = useCallback((f: (s: DefsState) => DefsState) => {
    now.current = f(now.current);
    setSt(now.current);
  }, []);

  // Lists asked so far: one answered after a newer one is stale (entering a tab and a save's refresh both out).
  const lists = useRef(0);
  const loadList = useCallback(() => {
    const seq = ++lists.current;
    const latest = () => seq === lists.current;
    client.definitions().then(
      (list) => latest() && update((s) => ({ ...s, list })),
      // egui keeps what it had and toasts; a tab with no list yet stays on Loading….
      (e: unknown) => latest() && say("error", `Could not load the orchestrators and specialists: ${message(e)}`),
    );
  }, [client, update, say]);

  // Per key, bumped as a save goes out and as it lands: a read sent before either may answer with the file the save
  // replaced, so its answer is dropped.
  const saves = useRef<Record<DefKey, number>>({});
  const bump = (key: DefKey) => (saves.current[key] = (saves.current[key] ?? 0) + 1);

  const fail = (key: DefKey) => (e: unknown) => update((s) => ({ ...s, errors: { ...s.errors, [key]: message(e) } }));

  const open = (kind: DefKind, name: string) => {
    const key = defKey(kind, name);
    const asked = saves.current[key];
    const fresh = () => saves.current[key] === asked;
    update((s) => ({ ...s, selected: { kind, name } }));
    client.definition(kind, name).then(
      (file) => fresh() && update((s) => loaded(s, key, file)),
      (e: unknown) => fresh() && fail(key)(e),
    );
  };

  return {
    st,
    loadList,
    open,
    edit: (key: DefKey, text: string) => update((s) => ({ ...s, drafts: { ...s.drafts, [key]: text } })),
    override: (key: DefKey) => {
      const file = now.current.files[key];
      if (file) update((s) => ({ ...s, drafts: { ...s.drafts, [key]: file.text } }));
    },
    save: (kind: DefKind, name: string) => {
      const key = defKey(kind, name);
      const text = now.current.drafts[key];
      if (now.current.saving || text === undefined) return;
      bump(key);
      update((s) => ({ ...s, saving: { key, text }, errors: without(s.errors, key) }));
      client.saveDefinition(kind, name, text).then(
        () => {
          bump(key);
          update(saved);
          say("success", `${name} saved`);
          loadList();
        },
        // The message goes under the editor; the draft stays to be fixed.
        (e: unknown) => update((s) => ({ ...s, saving: null, errors: { ...s.errors, [key]: message(e) } })),
      );
    },
    revert: (key: DefKey) =>
      update((s) => {
        const file = s.files[key];
        if (!file) return s;
        const drafts = file.builtin ? without(s.drafts, key) : { ...s.drafts, [key]: file.text };
        return { ...s, drafts, errors: without(s.errors, key) };
      }),
    /** Asked after the confirm: deletes the user's copy, then reads the built-in again if it is still open. */
    reset: (kind: DefKind, name: string) => {
      const key = defKey(kind, name);
      client.resetDefinition(kind, name).then(() => {
        update((s) => reset(s, key));
        say("success", `${name} reset to built-in`);
        const sel = now.current.selected;
        if (sel?.kind === kind && sel.name === name) open(kind, name);
        loadList();
      }, fail(key));
    },
  };
}

export type DefinitionsApi = ReturnType<typeof useDefinitions>;
