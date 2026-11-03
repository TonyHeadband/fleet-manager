import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { InternalBranches } from "../api/generated/InternalBranches";
import type { InternalCommit } from "../api/generated/InternalCommit";
import type { InternalRemote } from "../api/generated/InternalRemote";

// A port of crates/fleet-gui/src/views/management/internal.rs's InternalState and of its calls in app/products.rs:
// the product's private notes repo, read only. Every answer is for a product, branch and path still shown; a late
// answer for anything else is dropped.

export interface InternalState {
  /** The product the rest belongs to. */
  product: string | null;
  /** The section is open; it opens closed for each product. */
  open: boolean;
  /** What was shown before `forget`, chosen again if the new answers still have it. */
  keepBranch: string | null;
  keepFile: string | null;
  branches: string[] | null;
  remote: InternalRemote;
  branch: string | null;
  files: string[] | null;
  file: string | null;
  text: string | null;
  last: InternalCommit | null;
  error: string | null;
}

const fresh = (product: string | null): InternalState => ({
  product,
  open: false,
  keepBranch: null,
  keepFile: null,
  branches: null,
  remote: { url: null, unpushed: 0, error: null, parked: [] },
  branch: null,
  files: null,
  file: null,
  text: null,
  last: null,
  error: null,
});

/** Drops every answer so the section fetches again, remembering the branch and file shown. */
export const forget = (st: InternalState): InternalState => ({
  ...fresh(st.product),
  open: st.open,
  keepBranch: st.branch ?? st.keepBranch,
  keepFile: st.file ?? st.keepFile,
});

/** Switches branch: what belonged to the old one goes. */
const selectBranch = (st: InternalState, branch: string): InternalState => ({
  ...st,
  branch,
  files: null,
  file: null,
  text: null,
  last: null,
  error: null,
});

/** A refusal is the daemon's message. */
type Answer<T> = T | { error: string };
const failed = <T>(a: Answer<T>): a is { error: string } => typeof a === "object" && a !== null && "error" in a && !Array.isArray(a);

/**
 * The branches arrived. Returns the branch whose files to fetch: the one shown before a refresh if it is still
 * there, else the first, `main` leading.
 */
export function branchesDone(st: InternalState, product: string, result: Answer<InternalBranches>): [InternalState, string | null] {
  if (st.product !== product || st.branches !== null) return [st, null];
  if (failed(result)) return [{ ...st, error: result.error }, null];
  // The daemon's order is by name; main leads.
  const branches = [...result.branches].sort((a, b) => Number(a !== "main") - Number(b !== "main"));
  const kept = st.keepBranch !== null && branches.includes(st.keepBranch) ? st.keepBranch : null;
  const first = kept ?? branches[0] ?? null;
  const next = { ...st, remote: result.remote, branches, keepBranch: null, keepFile: kept === null ? null : st.keepFile };
  return [first === null ? next : selectBranch(next, first), first];
}

/** The files of a branch arrived. Returns the file whose text to fetch: the one shown before a refresh, if still there. */
export function filesDone(st: InternalState, product: string, branch: string, result: Answer<string[]>): [InternalState, string | null] {
  if (st.product !== product || st.branch !== branch) return [st, null];
  if (failed(result)) return [{ ...st, keepFile: null, error: result.error }, null];
  const kept = st.keepFile !== null && result.includes(st.keepFile) ? st.keepFile : null;
  return [{ ...st, keepFile: null, files: result, file: kept }, kept];
}

export function fileDone(
  st: InternalState,
  product: string,
  branch: string,
  path: string,
  result: Answer<{ text: string; last: InternalCommit | null }>,
): InternalState {
  if (st.product !== product || st.branch !== branch || st.file !== path) return st;
  if (failed(result)) return { ...st, error: result.error };
  return { ...st, text: result.text, last: result.last };
}

const refusal = (e: unknown) => ({ error: e instanceof Error ? e.message : String(e) });

/** The section's state and calls for `product`; another product starts afresh, closed. */
export function useInternal(client: FleetClient, product: string | null) {
  const [st, setSt] = useState(() => fresh(product));
  const now = useRef(st);
  const update = useCallback((next: InternalState) => {
    now.current = next;
    setSt(next);
  }, []);
  if (now.current.product !== product) update(fresh(product));

  const fetchFile = (p: string, branch: string, path: string) =>
    client.internalFile(p, branch, path).then(
      // The commit line is extra: a history that fails leaves the text without it.
      async (text) => {
        const last = await client.internalHistory(p, branch, path).then(
          (c) => c[0] ?? null,
          () => null,
        );
        update(fileDone(now.current, p, branch, path, { text, last }));
      },
      (e: unknown) => update(fileDone(now.current, p, branch, path, refusal(e))),
    );
  const fetchFiles = (p: string, branch: string) =>
    client
      .internalFiles(p, branch)
      .catch(refusal)
      .then((result) => {
        const [next, file] = filesDone(now.current, p, branch, result);
        update(next);
        if (file !== null) void fetchFile(p, branch, file);
      });
  const fetchBranches = (p: string) =>
    client
      .internalBranches(p)
      .catch(refusal)
      .then((result) => {
        const [next, branch] = branchesDone(now.current, p, result);
        update(next);
        if (branch !== null) void fetchFiles(p, branch);
      });

  return {
    st,
    open: () => {
      if (product === null) return;
      update({ ...now.current, open: true });
      void fetchBranches(product);
    },
    /** Closed: the next open fetches afresh, so a task started meanwhile shows up. */
    close: () => update({ ...forget(now.current), open: false }),
    refresh: () => {
      if (product === null) return;
      update(forget(now.current));
      void fetchBranches(product);
    },
    pickBranch: (branch: string) => {
      if (product === null || branch === now.current.branch) return;
      update(selectBranch(now.current, branch));
      void fetchFiles(product, branch);
    },
    pickFile: (path: string) => {
      const { branch, file } = now.current;
      if (product === null || branch === null || path === file) return;
      update({ ...now.current, file: path, text: null, last: null, error: null });
      void fetchFile(product, branch, path);
    },
  };
}

export type InternalApi = ReturnType<typeof useInternal>;
