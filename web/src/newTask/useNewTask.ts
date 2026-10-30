import { useEffect, useRef, useState } from "react";
import { FleetError, type FleetClient } from "../api/client";
import type { DefInfo } from "../api/generated/DefInfo";
import type { Product } from "../api/generated/Product";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { Notice } from "../shell/messages";
import { emptyDraft, keptRelease, picks, resetChoices, sentOk, setPersona, wantsLocal, type Draft, type Listings } from "./model";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * New task's state across openings, as Lite keeps its `SpawnState` in the app: what was typed survives a close, the
 * repos are asked for under the dialog's persona, and each local folder's checkouts are listed once per opening.
 */
export function useNewTask(client: FleetClient, say: (notice: Notice, text: string, agentId?: string) => void, product: Product | undefined, reloadRepos: () => void) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [repos, setRepos] = useState<RepoInfo[]>([]);
  const [reposError, setReposError] = useState<string | null>(null);
  const [defs, setDefs] = useState<DefInfo[]>([]);
  const [listings, setListings] = useState<Listings>({});
  // Read by answers that land later: the dialog's state as it is now, not as it was when they were asked.
  const now = useRef({ open, draft });
  now.current = { open, draft };
  // A listing asked for in an earlier opening, or a repos answer for another persona, is dropped.
  const opening = useRef(0);
  const reposAsked = useRef(0);

  const fetchRepos = (persona: string) => {
    if (!product) return;
    const n = ++reposAsked.current;
    client.repos(product.id).then(
      (list) => {
        if (n !== reposAsked.current || now.current.draft.persona !== persona) return;
        setRepos(list);
        setReposError(null);
        // A start picked for a name may not mean the same checkout now.
        setDraft((d) => ({ ...d, starts: {} }));
      },
      (e: unknown) => n === reposAsked.current && setReposError(`Could not load the repos: ${message(e)}`),
    );
  };

  const listCheckouts = (name: string, path: string) => {
    if (!product) return;
    const n = opening.current;
    setListings((l) => ({ ...l, [path]: { state: "loading" } }));
    client.checkouts(product.id, name).then(
      (checkouts) => n === opening.current && setListings((l) => ({ ...l, [path]: { state: "ready", checkouts } })),
      (e: unknown) => n === opening.current && setListings((l) => ({ ...l, [path]: { state: "failed", error: message(e) } })),
    );
  };

  // A local start lists its folder's checkouts when it first shows, as Lite's starts_section asks its cache.
  useEffect(() => {
    if (!open) return;
    for (const name of draft.chosen) {
      const path = repos.find((r) => r.name === name)?.path;
      if (path && wantsLocal(repos, draft, name) && !listings[path]) listCheckouts(name, path);
    }
  });

  // A release that is gone (released, or another product) is not sent under the Backlog label.
  const release = keptRelease(product, draft.release);
  useEffect(() => {
    if (release !== draft.release) setDraft((d) => ({ ...d, release }));
  }, [release, draft.release]);

  /** Opens under the product's default persona if ready, else the active one, else the first ready one (`spawnPersona`). */
  const openDialog = (persona: string) => {
    opening.current++;
    setListings({});
    setRepos((r) => (persona === draft.persona ? r : []));
    setDraft((d) => resetChoices(setPersona(d, persona)));
    setReposError(null);
    setOpen(true);
    now.current = { open: true, draft: { ...draft, persona } };
    fetchRepos(persona);
    client.definitions().then(setDefs, () => {});
  };

  const changePersona = (id: string) => {
    if (id === draft.persona) return;
    setRepos([]);
    setReposError(null);
    setDraft((d) => setPersona(d, id));
    now.current = { ...now.current, draft: { ...draft, persona: id } };
    fetchRepos(id);
  };

  const retry = (name: string, path: string) => listCheckouts(name, path);

  const start = () => {
    if (!product || draft.busy) return;
    const d = draft;
    setDraft({ ...d, busy: true, error: null, refusal: null, sent: d.prompt });
    client
      .spawn({
        actor: "local",
        product: product.id,
        release: keptRelease(product, d.release),
        repos: picks(repos, d, listings),
        prompt: d.prompt,
        slug: null,
        title: d.title.trim() === "" ? null : d.title,
        persona: d.persona,
        kind: d.role === null ? "solo" : "orchestrator",
        role: d.role,
      })
      .then(
        (started) => {
          // After the success line, so the footer ends on what still needs a look, as in Lite.
          say("success", "Agent started", started.agent.id);
          for (const w of started.warnings) say("warning", w, started.agent.id);
          reloadRepos();
          const cur = now.current.draft;
          // A goal reopened and changed while the spawn was on its way is kept, and so is the dialog.
          if (cur.prompt === d.prompt) {
            setOpen(false);
            setDraft(sentOk({ ...cur, busy: false, error: null, refusal: null, sent: null, prompt: "", title: "", chosen: [] }));
          } else {
            setDraft({ ...cur, busy: false, error: null, refusal: null, sent: null });
          }
        },
        (e: unknown) => {
          // Closed while in flight: the dialog's error line is out of sight, so the footer says it.
          if (!now.current.open) say("error", `Could not start the agent: ${message(e)}`);
          setDraft((cur) => ({ ...cur, busy: false, error: message(e), refusal: e instanceof FleetError ? (e.tool ?? null) : null }));
        },
      );
  };

  return {
    open,
    draft,
    setDraft,
    repos,
    reposError,
    orchestrators: defs.filter((d) => d.kind === "orchestrator" && !d.error).map((d) => d.name),
    listings,
    openDialog,
    close: () => setOpen(false),
    changePersona,
    retry,
    start,
  };
}
