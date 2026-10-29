import { useMemo, useState } from "react";
import { browserEventSource, type EventSourceFactory } from "./api/events";
import type { FleetClient } from "./api/client";
import type { ActionDef } from "./api/generated/ActionDef";
import type { Agent } from "./api/generated/Agent";
import { Board } from "./components/Board";
import { FormsPage } from "./components/FormsPage";
import { PreviewPane } from "./components/PreviewPane";
import { RepoFilter } from "./components/RepoFilter";
import { SpawnForm } from "./components/SpawnForm";
import { useFleet, useNow } from "./hooks";
import { activeProduct, newTaskBlocker, readChoice, spawnPersona, spawnRequest } from "./shell/choices";

interface Props {
  client: FleetClient;
  eventSource?: EventSourceFactory;
  nowSeconds?: () => number;
}

const wallClock = () => Math.floor(Date.now() / 1000);

/** `YYYY-MM-DD` of a Unix time on the local clock. */
function localDate(seconds: number) {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function App({ client, eventSource = browserEventSource, nowSeconds = wallClock }: Props) {
  const { snapshot, schema, repos: repoInfos, personas, products, connected, error } = useFleet(client, eventSource);
  const product = activeProduct(products, readChoice("fleet.product"));
  const repos = useMemo(() => repoInfos.filter((r) => product === null || r.product === product).map((r) => r.name), [repoInfos, product]);
  const now = useNow(nowSeconds);
  const [filter, setFilter] = useState<string[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [page, setPage] = useState<"board" | "forms">(() => (window.location.hash === "#forms" ? "forms" : "board"));

  const allRepos = useMemo(() => [...new Set([...repos, ...snapshot.agents.flatMap((a) => a.workspaces.map((w) => w.repo))])], [repos, snapshot.agents]);
  // An agent shows when any of its workspaces is in a selected repo.
  const shown = filter.length === 0 ? snapshot.agents : snapshot.agents.filter((a) => a.workspaces.some((w) => filter.includes(w.repo)));
  const open = snapshot.agents.find((a) => a.id === openId);

  const run = async (action: ActionDef, agent: Agent, input?: string) => {
    setProblem(null);
    try {
      await client.run(action, agent, input);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="app">
      <header className="top">
        <h1>fleet-manager</h1>
        <span className="conn" data-connected={connected}>
          {connected ? "live" : "reconnecting…"}
        </span>
        <nav className="pages">
          <a href="#" aria-current={page === "board" ? "page" : undefined} onClick={() => setPage("board")}>
            Board
          </a>
          <a href="#forms" aria-current={page === "forms" ? "page" : undefined} onClick={() => setPage("forms")}>
            Forms
          </a>
        </nav>
        {page === "board" && <RepoFilter repos={allRepos} selected={filter} onChange={setFilter} />}
      </header>
      {page === "forms" ? (
        schema ? <FormsPage client={client} schema={schema} today={localDate(now)} /> : <p className="loading">Loading…</p>
      ) : (
        <>
          <SpawnForm
            repos={repos}
            onSpawn={async (repo, prompt) => {
              const persona = product && spawnPersona(personas, products, product, readChoice("fleet.persona"));
              if (!product || !persona) throw new Error(newTaskBlocker(personas, products, product) ?? "Pick a product");
              await client.spawn(spawnRequest(product, persona, [repo], prompt));
            }}
          />
          {(error ?? problem) && <p role="alert">{error ?? problem}</p>}
          {schema ? (
            <div className={open ? "with-preview" : undefined}>
              <Board schema={schema} agents={shown} now={now} onRun={run} onOpen={(a) => setOpenId(a.id)} />
              {open && <PreviewPane baseUrl={client.baseUrl} agentId={open.id} title={open.goal} factory={eventSource} loadDiff={() => client.diff(open.id)} onClose={() => setOpenId(null)} />}
            </div>
          ) : (
            <p className="loading">Loading…</p>
          )}
        </>
      )}
    </div>
  );
}
