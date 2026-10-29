import { useEffect, useMemo, useState } from "react";
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
import { activeProduct, newTaskBlocker, readChoice, spawnPersona, writeChoice } from "./shell/choices";
import { Drawer } from "./shell/Drawer";
import { DetailPanel } from "./shell/DetailPanel";
import { Footer, useMessages } from "./shell/Footer";
import { layoutOf, usePanel, useWidth } from "./shell/panel";
import { useRoute } from "./shell/routes";
import { Shell } from "./shell/Shell";
import { Strip } from "./shell/Strip";

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

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ client, eventSource = browserEventSource, nowSeconds = wallClock }: Props) {
  const { snapshot, schema, repos: repoInfos, personas, products, tasks, connected, error } = useFleet(client, eventSource);
  const now = useNow(nowSeconds);
  const route = useRoute();
  const { messages, say, click } = useMessages();
  const [filter, setFilter] = useState<string[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const [spawning, setSpawning] = useState(false);
  const [savedProduct, setSavedProduct] = useState(() => readChoice("fleet.product"));
  const [savedPersona, setSavedPersona] = useState(() => readChoice("fleet.persona"));
  const panel = usePanel();
  const [bodyRef, bodyWidth] = useWidth<HTMLDivElement>();

  useEffect(() => {
    if (error) say("error", error);
  }, [error, say]);

  // As in Lite: a saved persona that is no longer a ready one gives way to the first ready one, which is saved.
  const persona = personas.state !== "ready" ? null : personas.value.some((p) => p.ready && p.id === savedPersona) ? savedPersona : (personas.value.find((p) => p.ready)?.id ?? null);
  useEffect(() => {
    if (personas.state !== "ready" || persona === savedPersona) return;
    setSavedPersona(persona);
    writeChoice("fleet.persona", persona);
  }, [personas.state, persona, savedPersona]);

  const product = activeProduct(products, savedProduct);
  // With no product selected there is nothing to narrow by, and the whole board shows, as in Lite.
  const scoped = useMemo(() => (product === null ? snapshot.agents : snapshot.agents.filter((a) => a.product === product)), [snapshot.agents, product]);
  const repoNames = useMemo(() => repoInfos.filter((r) => product === null || r.product === product).map((r) => r.name), [repoInfos, product]);
  const repoCounts = useMemo(() => {
    const names = [...new Set([...repoNames, ...scoped.flatMap((a) => a.workspaces.map((w) => w.repo))])];
    return names.map((r) => [r, scoped.filter((a) => a.workspaces.some((w) => w.repo === r)).length] as [string, number]);
  }, [repoNames, scoped]);
  // An agent shows when any of its workspaces is in a selected repo.
  const shown = filter.length === 0 ? scoped : scoped.filter((a) => a.workspaces.some((w) => filter.includes(w.repo)));
  const open = snapshot.agents.find((a) => a.id === openId);
  const blocked = newTaskBlocker(personas, products, savedProduct);
  const stoppedTone = schema?.columns.find((c) => c.statuses.includes("stopped"))?.tone ?? "muted";

  const run = async (action: ActionDef, agent: Agent, input?: string) => {
    try {
      await client.run(action, agent, input);
    } catch (e) {
      say("error", message(e), agent.id);
    }
  };

  const drawer = (
    <Drawer
      route={route}
      connected={connected}
      agents={snapshot.agents.length}
      needYou={snapshot.agents.filter((a) => a.status === "needs_you").length}
      personas={personas}
      persona={persona}
      onPersona={(id) => {
        setSavedPersona(id);
        writeChoice("fleet.persona", id);
      }}
      products={products}
      product={product}
      onProduct={(id) => {
        setSavedProduct(id);
        writeChoice("fleet.product", id);
        setFilter([]);
      }}
      newTaskBlocked={blocked}
      onNewTask={() => setSpawning(true)}
    />
  );
  const spawnAs = product === null ? null : spawnPersona(personas, products, product, persona);
  const footer = <Footer messages={messages} onClick={click} onSelect={setOpenId} />;
  const dialog = spawning && product !== null && spawnAs !== null && (
    <SpawnForm
      product={product}
      persona={spawnAs}
      repos={repoInfos.filter((r) => r.product === product)}
      onSpawn={async (req) => {
        await client.spawn(req);
        setSpawning(false);
      }}
      onClose={() => setSpawning(false)}
    />
  );

  if (route === "forms") {
    return (
      <Shell drawer={drawer} header={<h1 className="view-title">Forms</h1>} footer={footer} dialog={dialog}>
        {schema ? <FormsPage client={client} schema={schema} today={localDate(now)} /> : <p className="loading">Loading…</p>}
      </Shell>
    );
  }

  return (
    <Shell
      drawer={drawer}
      header={
        <>
          <h1 className="view-title">Kanban</h1>
          <RepoFilter repos={repoCounts} selected={filter} onChange={setFilter} />
        </>
      }
      strip={<Strip tasks={tasks} stoppedTone={stoppedTone} />}
      footer={footer}
      dialog={dialog}
      bodyRef={bodyRef}
      covered={!!open && layoutOf(panel, bodyWidth).kind === "full"}
      panel={
        open && (
          <DetailPanel title={open.goal} panel={panel} boardWidth={bodyWidth} onClose={() => setOpenId(null)}>
            <PreviewPane baseUrl={client.baseUrl} agentId={open.id} factory={eventSource} loadDiff={() => client.diff(open.id)} />
          </DetailPanel>
        )
      }
    >
      {schema ? <Board schema={schema} agents={shown} now={now} onRun={run} onOpen={(a) => setOpenId(a.id)} /> : <p className="loading">Loading…</p>}
    </Shell>
  );
}
