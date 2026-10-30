import { useCallback, useEffect, useRef, useState } from "react";
import { boardReducer } from "./api/board";
import type { FleetClient } from "./api/client";
import { browserEventSource, subscribe, type EventSourceFactory } from "./api/events";
import type { Persona } from "./api/generated/Persona";
import type { Product } from "./api/generated/Product";
import type { RepoInfo } from "./api/generated/RepoInfo";
import type { Snapshot } from "./api/generated/Snapshot";
import type { TaskList } from "./api/generated/TaskList";
import type { UiSchema } from "./api/generated/UiSchema";
import type { Load } from "./shell/choices";

/** How often the tasks list is fetched for the strip, and a failed persona or product list is asked for again, as Lite does. */
export const TASKS_EVERY_MS = 10_000;

export interface Fleet {
  snapshot: Snapshot;
  /** The first snapshot has come. */
  loaded: boolean;
  schema: UiSchema | null;
  repos: RepoInfo[];
  /** Asks for the repos again: a grant or an add changes them. */
  reloadRepos: () => void;
  personas: Load<Persona[]>;
  /** Asks for the personas again: the setup created or deleted one. */
  reloadPersonas: () => void;
  products: Load<Product[]>;
  tasks: TaskList | null;
  /** The tasks list as the Tasks view needs it: loading, failed, or shown with the last refresh's error. */
  taskLoad: Load<TaskList>;
  reloadTasks: () => void;
  connected: boolean;
  error: string | null;
}

/**
 * `fetch` now and again every `TASKS_EVERY_MS` while it fails; `poll` keeps fetching after success too. As Lite's
 * `fetch_tasks`: one request at a time (a tick or `reload` meanwhile asks for one more when it lands, so a stalled
 * daemon is not buried in requests), and only the latest request's answer is applied, so an old one never lands last.
 */
export function useLoad<T>(fetch: () => Promise<T>, poll = false): [Load<T>, () => void] {
  const [load, setLoad] = useState<Load<T>>({ state: "loading" });
  const current = useRef(fetch);
  const latest = useRef(0);
  const inFlight = useRef(false);
  const stale = useRef(false);
  const loaded = useRef(false);
  loaded.current = load.state === "ready";

  const ask = useCallback(() => {
    if (inFlight.current) {
      stale.current = true;
      return;
    }
    inFlight.current = true;
    const n = ++latest.current;
    const landed = () => {
      if (n !== latest.current) return;
      inFlight.current = false;
      if (stale.current) {
        stale.current = false;
        ask();
      }
    };
    current.current().then(
      (value) => {
        if (n === latest.current) setLoad({ state: "ready", value });
        landed();
      },
      (e: unknown) => {
        // A failed refresh of a list already shown keeps it, and says why.
        const error = e instanceof Error ? e.message : String(e);
        if (n === latest.current)
          setLoad((prev) => (prev.state === "ready" ? { state: "ready", value: prev.value, error } : { state: "failed", error }));
        landed();
      },
    );
  }, []);

  useEffect(() => {
    // Another fetch (another daemon) starts afresh: whatever the old one still has out is ignored when it lands.
    current.current = fetch;
    latest.current++;
    inFlight.current = false;
    stale.current = false;
    ask();
    const t = setInterval(() => {
      if (!poll && loaded.current) clearInterval(t);
      else ask();
    }, TASKS_EVERY_MS);
    return () => {
      clearInterval(t);
      latest.current++;
    };
  }, [fetch, poll, ask]);

  return [load, ask];
}

export function useFleet(client: FleetClient, factory: EventSourceFactory = browserEventSource): Fleet {
  const [snapshot, setSnapshot] = useState<Snapshot>({ seq: 0, agents: [] });
  const [loaded, setLoaded] = useState(false);
  // Retried like the lists below: a page opened while the daemon is down fills in once it answers.
  const [schema, reloadSchema] = useLoad(useCallback(() => client.uiSchema(), [client]));
  const [repos, reloadRepos] = useLoad(useCallback(() => client.repos(), [client]));
  // The persona ids an unknown agent persona asked the list for: settled by the answer, forgotten on a failure so the
  // next stream message asks again (Lite's `personas_refetching` and `personas_refetched`).
  const refetching = useRef<string[]>([]);
  const refetched = useRef(new Set<string>());
  const [personas, reloadPersonas] = useLoad(
    useCallback(
      () =>
        client.personas().then(
          (l) => {
            for (const id of refetching.current.splice(0)) refetched.current.add(id);
            return l.personas;
          },
          (e: unknown) => {
            refetching.current = [];
            throw e;
          },
        ),
      [client],
    ),
  );
  const [products] = useLoad(useCallback(() => client.products().then((l) => l.products), [client]));
  const [tasks, reloadTasks] = useLoad(
    useCallback(() => client.tasks(), [client]),
    true,
  );
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    // As Lite's `handle_stream`: a snapshot, or an agent that comes, goes or stops, changes the tasks list.
    const stop = subscribe(
      client.baseUrl,
      (s) => {
        setSnapshot(s);
        setLoaded(true);
        reloadTasks();
      },
      (e) => {
        setSnapshot((prev) => boardReducer(prev, e));
        const k = e.kind;
        if (k.type === "agent_created" || k.type === "agent_removed" || (k.type === "status_changed" && k.status === "stopped"))
          reloadTasks();
      },
      factory,
      setConnected,
      // A daemon that came back may be a newer one, with another schema or repos.
      () => {
        reloadSchema();
        reloadRepos();
      },
    );
    return stop;
  }, [client, factory, reloadSchema, reloadRepos, reloadTasks]);

  // An agent under a persona the list lacks (made since it loaded) asks for the list again, once per id.
  useEffect(() => {
    if (personas.state !== "ready") return;
    const unknown = snapshot.agents
      .map((a) => a.persona)
      .find((p) => p && !personas.value.some((l) => l.id === p) && !refetched.current.has(p) && !refetching.current.includes(p));
    if (unknown) {
      refetching.current.push(unknown);
      reloadPersonas();
    }
  }, [snapshot, personas, reloadPersonas]);

  const failed = [schema, repos].find((l) => l.state === "failed");
  return {
    snapshot,
    loaded,
    schema: schema.state === "ready" ? schema.value : null,
    repos: repos.state === "ready" ? repos.value : [],
    reloadRepos,
    error: failed?.state === "failed" ? (failed.error ?? null) : null,
    personas,
    reloadPersonas,
    products,
    tasks: tasks.state === "ready" ? tasks.value : null,
    taskLoad: tasks,
    reloadTasks,
    connected,
  };
}

/** Wall clock in unix seconds, refreshed so "waiting 5m" keeps counting. */
export function useNow(nowSeconds: () => number, everyMs = 15_000): number {
  const [now, setNow] = useState(nowSeconds);
  useEffect(() => {
    const t = setInterval(() => setNow(nowSeconds()), everyMs);
    return () => clearInterval(t);
  }, [nowSeconds, everyMs]);
  return now;
}
