import { useCallback, useEffect, useState } from "react";
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
  schema: UiSchema | null;
  repos: RepoInfo[];
  personas: Load<Persona[]>;
  products: Load<Product[]>;
  tasks: TaskList | null;
  connected: boolean;
  error: string | null;
}

/** `fetch` now and again every `TASKS_EVERY_MS` while it fails; `poll` keeps fetching after success too. */
function useLoad<T>(fetch: () => Promise<T>, poll = false): Load<T> {
  const [load, setLoad] = useState<Load<T>>({ state: "loading" });
  useEffect(() => {
    let live = true;
    let done = false;
    const once = () => {
      if (done && !poll) return;
      fetch().then(
        (value) => {
          done = true;
          if (live) setLoad({ state: "ready", value });
        },
        // A failed refresh of a list already shown keeps it.
        () => live && setLoad((prev) => (prev.state === "ready" ? prev : { state: "failed" })),
      );
    };
    once();
    const t = setInterval(once, TASKS_EVERY_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [fetch, poll]);
  return load;
}

export function useFleet(client: FleetClient, factory: EventSourceFactory = browserEventSource): Fleet {
  const [snapshot, setSnapshot] = useState<Snapshot>({ seq: 0, agents: [] });
  const [schema, setSchema] = useState<UiSchema | null>(null);
  const [repos, setRepos] = useState<RepoInfo[]>([]);
  const personas = useLoad(useCallback(() => client.personas().then((l) => l.personas), [client]));
  const products = useLoad(useCallback(() => client.products().then((l) => l.products), [client]));
  const tasks = useLoad(useCallback(() => client.tasks(), [client]), true);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([client.uiSchema(), client.repos()])
      .then(([s, p]) => {
        if (!live) return;
        setSchema(s);
        setRepos(p);
      })
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : String(e)));
    const stop = subscribe(
      client.baseUrl,
      (s) => setSnapshot(s),
      (e) => setSnapshot((prev) => boardReducer(prev, e)),
      factory,
      setConnected,
    );
    return () => {
      live = false;
      stop();
    };
  }, [client, factory]);

  return { snapshot, schema, repos, personas, products, tasks: tasks.state === "ready" ? tasks.value : null, connected, error };
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
