import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
import { Details, ReposRow } from "./panel/Details";
import { fleetPanel, useFleetViews } from "./panel/fleetViews";
import { Breakdown } from "./panel/Breakdown";
import { BREAKDOWN_DECISION, pending } from "./panel/breakdown";
import { useBreakdownDecisions } from "./panel/decisions";
import { REPO_REQUEST, RepoRequestBox, type RequestDraft } from "./panel/RepoRequestBox";
import { usableForm } from "./api/forms";
import { FleetSwitch } from "./components/FleetSwitch";
import { FormsPage } from "./components/FormsPage";
import { PreviewPane } from "./components/PreviewPane";
import { RepoFilter } from "./components/RepoFilter";
import { PersonaFilter } from "./components/PersonaFilter";
import { ReleaseFilter } from "./components/ReleaseFilter";
import { NewTaskDialog } from "./newTask/NewTaskDialog";
import { useNewTask } from "./newTask/useNewTask";
import type { Native } from "./native/native";
import type { WizardSteps } from "./startup/boot";
import { PersonaSetup } from "./startup/PersonaSetup";
import { UpdateNotice } from "./startup/UpdateNotice";
import { VersionOffer } from "./startup/VersionOffer";
import { CloseDialog } from "./shell/CloseDialog";
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
  /** The Tauri app's bridge to the machine it runs on; none in a browser. */
  native?: Native | null;
  /** What startup showed before the board, for the persona setup's steps. */
  wizard?: Wizard;
}

/** The startup steps already shown, the required tools continued without, and whether the daemon is on this machine. */
export interface Wizard {
  steps: WizardSteps;
  stillMissing: string[];
  local: boolean;
  /** This window started the daemon: no version offer, and a remembered stop is silent. */
  weStarted: boolean;
}

const NO_WIZARD: Wizard = { steps: { folders: false, requirements: false }, stillMissing: [], local: false, weStarted: false };

const wallClock = () => Math.floor(Date.now() / 1000);

/** `YYYY-MM-DD` of a Unix time on the local clock. */
function localDate(seconds: number) {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function App({ client, eventSource = browserEventSource, nowSeconds = wallClock, native = null, wizard = NO_WIZARD }: Props) {
  const {
    snapshot,
    loaded,
    schema,
    repos: repoInfos,
    reloadRepos,
    personas,
    reloadPersonas,
    products,
    tasks,
    connected,
    error,
  } = useFleet(client, eventSource);
  const now = useNow(nowSeconds);
  // The daemon's host, not the page's: in the app the page is the app's own.
  const daemonHost = new URL(client.baseUrl || window.location.href, window.location.href).hostname;
  const route = useRoute();
  const { messages, say, click } = useMessages();
  // The header filters; none is saved, as in Lite.
  const [filter, setFilter] = useState<string[]>([]);
  const [personaFilter, setPersonaFilter] = useState<string[]>([]);
  const [releaseFilter, setReleaseFilter] = useState<ReleaseFilterValue>({ type: "all" });
  const [openId, setOpenId] = useState<string | null>(null);
  const [display, setDisplay] = useState<FleetDisplay>(readFleetDisplay);
  // The task-level Stop waiting on its confirm: the task, the count of its running fleet, and whether its lead runs.
  const [stopping, setStopping] = useState<{ task: string; fleet: number; leadRuns: boolean; name: string } | null>(null);
  // Each agent's unsent answer, kept while the panel moves between cards, as Lite's per-agent drafts.
  const [replies, setReplies] = useState<Record<string, string>>({});
  // Repo request drafts by agent, each for the request it was started on: a request for another repo starts afresh.
  const [requests, setRequests] = useState<Record<string, RequestDraft & { forRepo: string }>>({});
  const [decidingRequest, setDecidingRequest] = useState<ReadonlySet<string>>(new Set());
  // Read before a render catches up, so a double press sends once.
  const requestInFlight = useRef(new Set<string>());
  const [sending, setSending] = useState<ReadonlySet<string>>(new Set());
  const [savedProduct, setSavedProduct] = useState(() => readChoice("fleet.product"));
  const [savedPersona, setSavedPersona] = useState(() => readChoice("fleet.persona"));
  const panel = usePanel();
  const [bodyRef, bodyWidth] = useWidth<HTMLDivElement>();

  useEffect(() => {
    if (error) say("error", error);
  }, [error, say]);

  // As Lite's `active_listed` and `adopt_active`: the saved persona holds until a loaded list no longer has it, ready
  // or not; one no longer listed gives way to the first ready one, if any, which is saved.
  const persona =
    personas.state !== "ready" || personas.value.some((p) => p.id === savedPersona)
      ? savedPersona
      : (personas.value.find((p) => p.ready)?.id ?? null);
  useEffect(() => {
    if (personas.state !== "ready" || persona === savedPersona) return;
    setSavedPersona(persona);
    writeChoice("fleet.persona", persona);
  }, [personas.state, persona, savedPersona]);

  const product = activeProduct(products, savedProduct);
  const productInfo = products.state === "ready" ? products.value.find((p) => p.id === product) : undefined;
  const personaList = personas.state === "ready" ? personas.value : [];
  const newTask = useNewTask(client, say, product, productInfo, reloadRepos);
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
  const shown = shownAgents(snapshot.agents, { product, release, repos: filter, personas: personaPicks, tasks: tasks?.tasks ?? [] });
  const tags = personaTags(shown, personaCounts);
  const open = snapshot.agents.find((a) => a.id === openId);
  const fleet = useFleetViews(client, say);
  const openFleet = open ? fleetPanel(snapshot.agents, open) : null;
  const openTask = open && openFleet?.type === "task" ? taskId(open) : null;
  // The open orchestrator's task, as its agents stand: opening it, and any change among them (Lite's
  // follow_fleet_event), asks for its view again.
  const taskState =
    openTask === null
      ? ""
      : snapshot.agents
          .filter((a) => taskId(a) === openTask)
          .map((a) => `${a.id}:${a.status}`)
          .join(" ");
  useEffect(() => {
    if (openTask !== null) fleet.fetch(openTask);
  }, [openTask, taskState, fleet.fetch]);
  const decisions = useBreakdownDecisions(client, fleet.views, fleet.fetch, say);
  const blocked = newTaskBlocker(personas, products, savedProduct);
  const breakdownTab = (lead: Agent, task: string) => {
    const load = fleet.views[task];
    const node = !load ? (
      <p className="note">Loading…</p>
    ) : load.error !== undefined ? (
      <p role="alert">{load.error}</p>
    ) : (
      <Breakdown
        view={load.view!}
        agents={snapshot.agents.filter((a) => taskId(a) === task)}
        nicknames={personaList.find((p) => p.id === lead.persona)?.nicknames ?? {}}
        pending={pending(load.view!)}
        busy={decisions.busy(task)}
        draft={decisions.draft(task)}
        onDraft={(t) => decisions.setDraft(task, t)}
        onApprove={() => decisions.decide(task, null)}
        onRequestChanges={(c) => decisions.decide(task, c)}
        onOpen={setOpenId}
        form={(schema && usableForm(schema, "breakdown_decision")) ?? BREAKDOWN_DECISION}
      />
    );
    return { id: "breakdown", label: "Breakdown", first: lead.blocked_reason === "breakdown", node };
  };
  const stoppedTone = schema?.columns.find((c) => c.statuses.includes("stopped"))?.tone ?? "muted";

  const run = async (action: ActionDef, agent: Agent, input?: string) => {
    try {
      await client.run(action, agent, input);
    } catch (e) {
      say("error", message(e), agent.id);
    }
  };

  // Lite's focus rule: a box takes the cursor only on the panel's first draw for an agent, so one that appears later
  // (the agent starts asking while its panel is open) never pulls the cursor from wherever the user is. Set while
  // rendering the newly opened panel, cleared once that render has committed.
  const focusFor = useRef<string | null>(null);
  const lastOpened = useRef<string | null>(null);
  if (openId !== lastOpened.current) {
    lastOpened.current = openId;
    focusFor.current = openId;
  }
  useEffect(() => {
    focusFor.current = null;
  });
  const takesFocus = (id: string) => focusFor.current === id;

  const decideRequest = (id: string, grant: boolean, send: () => Promise<unknown>) => {
    if (requestInFlight.current.has(id)) return;
    requestInFlight.current.add(id);
    setDecidingRequest(new Set(requestInFlight.current));
    send()
      .then(
        () => {
          reloadRepos();
          if (grant) say("success", "Repo granted", id);
        },
        (e: unknown) => say("error", message(e), id),
      )
      .finally(() => {
        requestInFlight.current.delete(id);
        setDecidingRequest(new Set(requestInFlight.current));
      });
  };
  // A stopped agent's leftover request has nobody waiting on it, so it gets no box.
  const request = open && open.status !== "stopped" ? open.repo_request : null;
  const requestBox =
    open &&
    request &&
    (() => {
      const kept = requests[open.id];
      const draft = kept?.forRepo === request.repo ? kept : { target: request.repo, reason: "" };
      return (
        <RepoRequestBox
          key={open.id}
          request={request}
          productRepos={repoInfos.filter((r) => r.product === open.product).map((r) => r.name)}
          form={(schema && usableForm(schema, "repo_request")) ?? REPO_REQUEST}
          draft={draft}
          onDraft={(d) => setRequests((r) => ({ ...r, [open.id]: { ...d, forRepo: request.repo } }))}
          blocked={decidingRequest.has(open.id) ? "A decision is being sent" : null}
          focus={takesFocus(open.id)}
          onGrant={(repo) => decideRequest(open.id, true, () => client.grantRepo(open.id, repo))}
          onRefuse={(reason) => decideRequest(open.id, false, () => client.refuseRepo(open.id, reason))}
        />
      );
    })();

  // The open agent's first input action (Answer) gets the panel's reply box, unless a repo request takes its place.
  const replyAction = open && schema ? visibleActions(schema, open).find((a) => a.input !== null) : undefined;
  const reply =
    requestBox ||
    (open && replyAction && (
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
        focus={takesFocus(open.id)}
      />
    ));

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
      title={
        stopping.leadRuns
          ? stopping.fleet === 0
            ? "Stop the orchestrator?"
            : `Stop the orchestrator and ${fleetAgents(stopping.fleet)}?`
          : `Stop ${fleetAgents(stopping.fleet)}?`
      }
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
      onNewTask={() => newTask.openDialog(spawnAs ?? "")}
    />
  );
  const spawnAs = product === null ? null : spawnPersona(personas, products, product, persona);
  // As Lite's close_preview: the next card opens at the side, not where the last one left off.
  const closePanel = () => {
    setOpenId(null);
    panel.setChosenFull(false);
  };
  const footer = <Footer messages={messages} onClick={click} onSelect={setOpenId} />;
  // egui's wizard over the board while no persona is ready; it cannot be dismissed, and New task waits for it.
  const setup = personas.state === "ready" && !personas.value.some((p) => p.ready) && (
    <PersonaSetup
      client={client}
      native={native}
      local={wizard.local}
      steps={wizard.steps}
      stillMissing={wizard.stillMissing}
      draft={personas.value[0] ?? null}
      onFinish={reloadPersonas}
      onChanged={reloadPersonas}
    />
  );
  const dialog =
    setup ||
    (newTask.open && (
      <NewTaskDialog
        draft={newTask.draft}
        onDraft={newTask.setDraft}
        personas={personaList}
        product={productInfo}
        orchestrators={newTask.orchestrators}
        repos={newTask.repos}
        reposError={newTask.reposError}
        listings={newTask.listings}
        onPersona={newTask.changePersona}
        onRetry={newTask.retry}
        onStart={newTask.start}
        onClose={newTask.close}
      />
    ));

  // The app's own dialogs, over whatever the view shows: the close flow, the version offer, the update notice.
  const appDialogs = (main: ReactNode) => (
    <>
      {main}
      <CloseDialog native={native} agents={snapshot.agents} />
      <VersionOffer
        client={client}
        native={native}
        weStarted={wizard.weStarted}
        loaded={loaded}
        onRestarted={() => {
          void native?.setCloseArmed(false);
          window.location.reload();
        }}
      />
      <UpdateNotice native={native} blocked={!!main} />
    </>
  );

  if (route === "forms") {
    return (
      <Shell drawer={drawer} header={<h1 className="view-title">Forms</h1>} footer={footer} dialog={appDialogs(dialog)}>
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
      dialog={appDialogs(setup || stopDialog || dialog)}
      bodyRef={bodyRef}
      covered={!!open && layoutOf(panel, bodyWidth).kind === "full"}
      panel={
        open && (
          <DetailPanel
            agent={open}
            bands={
              schema && (
                <>
                  {openFleet?.type === "member" && (
                    <button type="button" className="link task-link" onClick={() => setOpenId(openFleet.lead)}>
                      {`↳ ${openFleet.task}`}
                    </button>
                  )}
                  <Details schema={schema} agent={open} now={now} />
                  <ReposRow agent={open} />
                </>
              )
            }
            panel={panel}
            boardWidth={bodyWidth}
            daemonHost={daemonHost}
            onTerminal={
              native && wizard.local ? () => native.openTerminal(open.id).catch((e: unknown) => say("error", message(e), open.id)) : null
            }
            onClose={closePanel}
            reply={reply}
          >
            <PreviewPane
              baseUrl={client.baseUrl}
              agentId={open.id}
              factory={eventSource}
              loadDiff={() => client.diff(open.id)}
              extraTabs={openTask !== null ? [breakdownTab(open, openTask)] : []}
            />
          </DetailPanel>
        )
      }
    >
      {schema ? (
        <Board
          schema={schema}
          agents={shown}
          now={now}
          selected={open?.id ?? null}
          tags={tags}
          display={display}
          prs={fleet.prs}
          personas={personaList}
          onStopTask={(task, fleet) => {
            const lead = leadOf(task);
            setStopping({ task, fleet, leadRuns: lead?.status !== "stopped", name: lead?.goal ?? task });
          }}
          onRestart={(task) => void restart(task)}
          onRun={run}
          onOpen={(a) => setOpenId(a.id)}
        />
      ) : (
        <p className="loading">Loading…</p>
      )}
    </Shell>
  );
}
