import { useEffect, useMemo, useState } from "react";
import { browserEventSource, type EventSourceFactory } from "./api/events";
import type { FleetClient } from "./api/client";
import type { ActionDef } from "./api/generated/ActionDef";
import type { Agent } from "./api/generated/Agent";
import { Board } from "./components/Board";
import { ReplyBox } from "./components/ReplyBox";
import { visibleActions } from "./api/schema";
import { kept, personaRows, personaTags, repoRows, shownAgents, type ReleaseFilter as ReleaseFilterValue } from "./board/filters";
import { groups, taskId, type FleetDisplay } from "./board/groups";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { FleetSwitch } from "./components/FleetSwitch";
import { FormsPage } from "./components/FormsPage";
import { PreviewPane } from "./components/PreviewPane";
import { RepoFilter } from "./components/RepoFilter";
import { PersonaFilter } from "./components/PersonaFilter";
import { ReleaseFilter } from "./components/ReleaseFilter";
import { SpawnForm } from "./components/SpawnForm";
import { useFleet, useNow } from "./hooks";
import { activeProduct, newTaskBlocker, readChoice, readFleetDisplay, spawnPersona, writeChoice } from "./shell/choices";
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
  // The header filters; none is saved, as in Lite.
  const [filter, setFilter] = useState<string[]>([]);
  const [personaFilter, setPersonaFilter] = useState<string[]>([]);
  const [releaseFilter, setReleaseFilter] = useState<ReleaseFilterValue>({ type: "all" });
  const [openId, setOpenId] = useState<string | null>(null);
  const [spawning, setSpawning] = useState(false);
  const [display, setDisplay] = useState<FleetDisplay>(readFleetDisplay);
  // The task-level Stop waiting on its confirm: the task, the count of its running fleet, and whether its lead runs.
  const [stopping, setStopping] = useState<{ task: string; fleet: number; leadRuns: boolean; name: string } | null>(null);
  // Each agent's unsent answer, kept while the panel moves between cards, as Lite's per-agent drafts.
  const [replies, setReplies] = useState<Record<string, string>>({});
  const [sending, setSending] = useState<ReadonlySet<string>>(new Set());
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
  const personaList = personas.state === "ready" ? personas.value : [];
  const repoCounts = useMemo(() => repoRows(repoInfos, snapshot.agents), [repoInfos, snapshot.agents]);
  const personaCounts = personaRows(personaList, snapshot.agents);
  const releases = (products.state === "ready" && products.value.find((p) => p.id === product)?.releases) || [];
  // A persona or release no longer offered (deleted, released) must not keep hiding cards out of sight, as in Lite.
  const personaPicks = personaFilter.filter((id) => personaCounts.some((r) => r.id === id));
  const release = kept(releaseFilter, releases);
  useEffect(() => {
    if (personaPicks.length !== personaFilter.length) setPersonaFilter(personaPicks);
  }, [personaPicks, personaFilter]);
  useEffect(() => {
    if (release !== releaseFilter) setReleaseFilter(release);
  }, [release, releaseFilter]);
  // With no product selected there is nothing to narrow by, and the whole board shows, as in Lite.
  const shown = shownAgents(groups(snapshot.agents), { product, release, repos: filter, personas: personaPicks, tasks: tasks?.tasks ?? [] });
  const tags = personaTags(shown, personaCounts);
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

  // The open agent's first input action (Answer) gets the panel's reply box.
  const replyAction = open && schema ? visibleActions(schema, open).find((a) => a.input !== null) : undefined;
  const reply = open && replyAction && (
    <ReplyBox
      key={open.id}
      label={replyAction.label}
      text={replies[open.id] ?? ""}
      onText={(text) => setReplies((r) => ({ ...r, [open.id]: text }))}
      busy={sending.has(open.id)}
      onSend={async (text) => {
        const id = open.id;
        setSending((s) => new Set(s).add(id));
        try {
          await client.run(replyAction, open, text);
          // As Lite: only what was sent is cleared, so text typed while it was on its way stays.
          setReplies((r) => (r[id] === text ? { ...r, [id]: "" } : r));
        } catch (e) {
          say("error", message(e), id);
        } finally {
          setSending((s) => {
            const next = new Set(s);
            next.delete(id);
            return next;
          });
        }
      }}
      focus
    />
  );

  // A task's name for the footer: its lead's goal while it is on the board, as Lite's `task_name`.
  const leadOf = (task: string) => groups(snapshot.agents).find((g) => taskId(g.lead) === task)?.lead;
  const stopAll = async (task: string, name: string) => {
    try {
      const { stopped } = await client.stopTask(task);
      say("success", `${name}: ${stopped} ${stopped === 1 ? "agent" : "agents"} stopped`);
    } catch (e) {
      say("error", `Could not stop ${name}: ${message(e)}`);
    }
  };
  const restart = async (task: string) => {
    const name = leadOf(task)?.goal ?? task;
    try {
      const started = await client.restartOrchestrator(task);
      say("success", "Orchestrator restarted", started.id);
    } catch (e) {
      say("error", `Could not restart the orchestrator of ${name}: ${message(e)}`);
    }
  };
  const fleetAgents = (n: number) => (n === 1 ? "1 fleet agent" : `${n} fleet agents`);
  const stopDialog = stopping && (
    <ConfirmDialog
      title={stopping.leadRuns ? (stopping.fleet === 0 ? "Stop the orchestrator?" : `Stop the orchestrator and ${fleetAgents(stopping.fleet)}?`) : `Stop ${fleetAgents(stopping.fleet)}?`}
      body="Their sessions end. Folders and branches stay, so the task can be archived later."
      go="Stop all"
      onGo={() => {
        setStopping(null);
        void stopAll(stopping.task, stopping.name);
      }}
      onCancel={() => setStopping(null)}
    />
  );

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
        // As Lite's `set_active_product`: only the release filter belongs to a product.
        setReleaseFilter({ type: "all" });
      }}
      newTaskBlocked={blocked}
      onNewTask={() => setSpawning(true)}
    />
  );
  const spawnAs = product === null ? null : spawnPersona(personas, products, product, persona);
  // As Lite's close_preview: the next card opens at the side, not where the last one left off.
  const closePanel = () => {
    setOpenId(null);
    panel.setChosenFull(false);
  };
  const footer = <Footer messages={messages} onClick={click} onSelect={setOpenId} />;
  const dialog = spawning && product !== null && spawnAs !== null && (
    <SpawnForm
      product={product}
      persona={spawnAs}
      repos={repoInfos.filter((r) => r.product === product)}
      onSpawn={async (req) => {
        const started = await client.spawn(req);
        // After the success line, so the footer ends on what still needs a look, as in Lite.
        say("success", "Agent started", started.agent.id);
        for (const w of started.warnings) say("warning", w, started.agent.id);
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
          <div className="filters">
            <RepoFilter repos={repoCounts} selected={filter} onChange={setFilter} />
            <PersonaFilter rows={personaCounts} selected={personaPicks} onChange={setPersonaFilter} />
            <ReleaseFilter releases={releases} value={release} onChange={setReleaseFilter} />
          </div>
          <FleetSwitch
            current={display}
            onPick={(d) => {
              setDisplay(d);
              writeChoice("fleet.display", d);
            }}
          />
        </>
      }
      strip={<Strip tasks={tasks} stoppedTone={stoppedTone} />}
      footer={footer}
      dialog={stopDialog || dialog}
      bodyRef={bodyRef}
      covered={!!open && layoutOf(panel, bodyWidth).kind === "full"}
      panel={
        open && (
          <DetailPanel title={open.goal} panel={panel} boardWidth={bodyWidth} onClose={closePanel} reply={reply}>
            <PreviewPane baseUrl={client.baseUrl} agentId={open.id} factory={eventSource} loadDiff={() => client.diff(open.id)} />
          </DetailPanel>
        )
      }
    >
      {schema ? <Board schema={schema} agents={shown} now={now} selected={open?.id ?? null} tags={tags} display={display}
          personas={personaList}
          onStopTask={(task, fleet) => {
            const lead = leadOf(task);
            setStopping({ task, fleet, leadRuns: lead?.status !== "stopped", name: lead?.goal ?? task });
          }}
          onRestart={(task) => void restart(task)}
          onRun={run} onOpen={(a) => setOpenId(a.id)} /> : <p className="loading">Loading…</p>}
    </Shell>
  );
}
