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
export function useNewTask(
  client: FleetClient,
  say: (notice: Notice, text: string, agentId?: string) => void,
  /** The active product's id: a saved one is enough to start a task, even while the products list cannot load. */
  productId: string | null,
  /** Its loaded description, for the releases. */
  product: Product | undefined,
  reloadRepos: () => void,
) {
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
    if (productId === null) return;
    const n = ++reposAsked.current;
    client.repos(productId).then(
      (list) => {
        if (n !== reposAsked.current || now.current.draft.persona !== persona) return;
        setRepos(list);
        setReposError(null);
        // A start picked for a name may not mean the same checkout now, and a pick the list lacks (another product's)
        // has no chip to untick, as Lite's `set_repos`.
        setDraft((d) => ({ ...d, chosen: d.chosen.filter((c) => list.some((r) => r.name === c)), starts: {} }));
      },
      (e: unknown) => n === reposAsked.current && setReposError(`Could not load the repos: ${message(e)}`),
    );
  };

  // The dialog's persona owns the orchestrators it offers; an answer for a persona since left is dropped.
  const fetchDefs = (persona: string) => {
    client.personaDefinitions(persona).then(
      (list) => now.current.draft.persona === persona && setDefs(list),
      () => {},
    );
  };

  const listCheckouts = (name: string, path: string) => {
    if (productId === null) return;
    const n = opening.current;
    setListings((l) => ({ ...l, [path]: { state: "loading" } }));
    client.checkouts(productId, name).then(
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
    // listCheckouts reads only the client, the product and the opening, none of which changes a listing's need.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, draft.chosen, draft.starts, repos, listings]);

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
    setDefs((d) => (persona === draft.persona ? d : []));
    setDraft((d) => resetChoices(setPersona(d, persona)));
    setReposError(null);
    setOpen(true);
    now.current = { open: true, draft: { ...draft, persona } };
    // As Lite's open path: the board's repos are asked for again, and the dialog's for its persona.
    reloadRepos();
    fetchRepos(persona);
    fetchDefs(persona);
  };

  const changePersona = (id: string) => {
    if (id === draft.persona) return;
    setRepos([]);
    setReposError(null);
    setDraft((d) => setPersona(d, id));
    now.current = { ...now.current, draft: { ...draft, persona: id } };
    fetchRepos(id);
    setDefs([]);
    fetchDefs(id);
  };

  const retry = (name: string, path: string) => listCheckouts(name, path);

  const start = () => {
    if (productId === null || draft.busy) return;
    const d = draft;
    setDraft({ ...d, busy: true, error: null, refusal: null, sent: d.prompt });
    client
      .spawn({
        actor: "local",
        product: productId,
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
          // A goal reopened and changed while the spawn was on its way is kept, and so is the dialog. The draft is
          // written from its latest state, so a keystroke not yet rendered is not lost.
          const sentGoal = (x: Draft) => x.prompt === d.prompt;
          setDraft((x) =>
            sentGoal(x)
              ? sentOk({ ...x, busy: false, error: null, refusal: null, sent: null, prompt: "", title: "", chosen: [] })
              : { ...x, busy: false, error: null, refusal: null, sent: null },
          );
          if (sentGoal(cur)) setOpen(false);
          // Lite's refresh_repos asks for the open dialog's repos too.
          else if (now.current.open) fetchRepos(cur.persona);
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
