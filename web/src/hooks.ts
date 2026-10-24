import { useEffect, useState } from "react";
import { boardReducer } from "./api/board";
import type { FleetClient } from "./api/client";
import { browserEventSource, subscribe, type EventSourceFactory } from "./api/events";
import type { Snapshot } from "./api/generated/Snapshot";
import type { UiSchema } from "./api/generated/UiSchema";

export interface Fleet {
  snapshot: Snapshot;
  schema: UiSchema | null;
  repos: string[];
  connected: boolean;
  error: string | null;
}

export function useFleet(client: FleetClient, factory: EventSourceFactory = browserEventSource): Fleet {
  const [snapshot, setSnapshot] = useState<Snapshot>({ seq: 0, agents: [] });
  const [schema, setSchema] = useState<UiSchema | null>(null);
  const [repos, setRepos] = useState<string[]>([]);
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

  return { snapshot, schema, repos, connected, error };
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
