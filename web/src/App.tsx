import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { browserEventSource, type EventSourceFactory } from "./api/events";
import type { FleetClient } from "./api/client";
import type { ActionDef } from "./api/generated/ActionDef";
import type { Agent } from "./api/generated/Agent";
import type { Persona } from "./api/generated/Persona";
import { Board } from "./components/Board";
import { ReplyBox } from "./components/ReplyBox";
import { stopAction, visibleActions } from "./api/schema";
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
import { PersonaSetup, type SetupMode } from "./startup/PersonaSetup";
import { UpdateNotice } from "./startup/UpdateNotice";
import { VersionOffer } from "./startup/VersionOffer";
import { CloseDialog } from "./shell/CloseDialog";
import { useFleet, useNow } from "./hooks";
import { activeProduct, newTaskBlocker, readChoice, readFleetDisplay, spawnPersona, writeChoice } from "./shell/choices";
import { Drawer } from "./shell/Drawer";
import { DetailPanel } from "./shell/DetailPanel";
import { Footer, useMessages } from "./shell/Footer";
import { carryNotice, takeCarriedNotice, type Notice } from "./shell/messages";
import { layoutOf, usePanel, useWidth } from "./shell/panel";
import {
  navigate,
  settingsHref,
  tasksRouteFromHash,
  useHash,
  useManagementTab,
  useRoute,
  useSettingsTab,
  useTasksRoute,
  type Route,
} from "./shell/routes";
import { currentPreferences, mergePreferences, usePreferences } from "./shell/preferences";
import { DaemonTab } from "./settings/DaemonTab";
import { Extras } from "./settings/Extras";
import { LogTab } from "./settings/LogTab";
import { loggedSay, useLog } from "./settings/log";
import { RelaunchedWizard } from "./settings/RelaunchedWizard";
import { useToolCheck } from "./startup/toolCheck";
import { FIRST_PICK, General } from "./settings/General";
import { useRestart } from "./settings/restart";
import { useDaemonSettings } from "./settings/daemonSettings";
import { SettingsView } from "./settings/SettingsView";
import { useAppVersion, useUpdateCheck } from "./startup/update";
import { Shell } from "./shell/Shell";
import { Strip } from "./shell/Strip";
import { Button } from "./ui/Button";
import { entryId, entryName, liveTasks } from "./tasks/model";
import { Passport } from "./tasks/Passport";
import { ArchiveDialog, useArchive } from "./tasks/ArchiveDialog";
import { TasksView } from "./tasks/TasksView";
import { ReleasesView } from "./releases/ReleasesView";
import { ManagementView } from "./management/ManagementView";
import { NotYet } from "./ui/NotYet";
import { Overview } from "./management/Overview";
import { Workflow } from "./management/Workflow";
import { useAgentConfig } from "./management/agentConfig";
import { Instructions } from "./management/Instructions";
import { AgentSettings } from "./management/AgentSettings";
import { DEF_TABS, usable, useDefinitions } from "./management/definitions";
import { Definitions } from "./management/Definitions";
import { usePersonasTab } from "./management/personas";
import { Personas } from "./management/Personas";
import { NewProduct } from "./management/NewProduct";
import { useProductsTab } from "./management/products";
import { Products } from "./management/Products";

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
    reloadSchema,
    repos: repoInfos,
    reloadRepos,
    personas,
    reloadPersonas,
    products,
    reloadProducts,
    tasks,
    taskLoad,
    reloadTasks,
    refreshTasks,
    connected,
    error,
  } = useFleet(client, eventSource);
  const now = useNow(nowSeconds);
  // The daemon's host, not the page's: in the app the page is the app's own.
  const daemonHost = new URL(client.baseUrl || window.location.href, window.location.href).hostname;
  const route = useRoute();
  const tasksRoute = useTasksRoute();
  const managementTab = useManagementTab();
  const agentConfig = useAgentConfig(client);
  const loadConfig = agentConfig.load;
  // Loaded when Management is first opened, not at start; a failed load is asked again on each way back in, by
  // view or by tab (egui's set_view and management_view).
  useEffect(() => {
    if (route === "management") loadConfig();
  }, [route, managementTab, loadConfig]);
  const { messages, say: show, click } = useMessages();
  // egui's notices are tracing events: each one is a log line too, in the Log tab and in gui.log.
  const say = useMemo(() => loggedSay(native, show), [native, show]);
  // For a choice saved that was not the person's pick (a persona adopted, a deleted product let go): only a failed
  // save is worth a word, where egui also says "Settings saved".
  const sayFailure = useCallback((notice: Notice, text: string) => notice !== "success" && say(notice, text), [say]);
  const definitions = useDefinitions(client, say);
  const personasTab = usePersonasTab(client, personas.state === "ready" ? personas.value : null, reloadPersonas, native, wizard.local, say);
  // Management's "+ New persona" and Resume open the setup in their modes, over the app.
  const [managedSetup, setManagedSetup] = useState<{ mode: Exclude<SetupMode, "wizard">; draft: Persona | null } | null>(null);
  const loadDefinitions = definitions.loadList;
  // Asked again on each way into a definitions tab, into Personas for its nickname fields and into Products for its
  // specialists and override keys, so files changed on disk show (egui asks at start and after a save or reset).
  useEffect(() => {
    if (route === "management" && (managementTab in DEF_TABS || managementTab === "personas" || managementTab === "products"))
      loadDefinitions();
  }, [route, managementTab, loadDefinitions]);
  // Where Tasks, Management and Settings were left, so the drawer and the strip open them there again (egui keeps
  // their state).
  const hash = useHash();
  const lastHref = useRef<Partial<Record<Route, string>>>({});
  if (route === "tasks" || route === "management" || route === "settings") lastHref.current[route] = hash || `#${route}`;
  const lastTasks = tasksRouteFromHash(lastHref.current.tasks ?? "");
  // Kept while the view is left, as egui's TasksState.
  const [showArchived, setShowArchived] = useState(false);
  // Overview's product filter, kept while the tab and the view are left, as egui's OverviewState.
  const [overviewShown, setOverviewShown] = useState<string | null>(null);
  // What the page before a reload said as it went ("Daemon restarted").
  useEffect(() => {
    const kept = takeCarriedNotice();
    if (kept) say(kept.notice, kept.text);
  }, [say]);
  // egui's first frame: why gui.log is not being written, when it is not.
  useEffect(() => {
    native?.logFileError().then((reason) => reason && say("warning", `Logging to file is off: ${reason}`), () => {});
  }, [native, say]);
  const settingsTab = useSettingsTab();
  const log = useLog(native, route === "settings" && settingsTab === "log");
  // The relaunched setup wizard, over whatever view opened it.
  const [wizardOpen, setWizardOpen] = useState(false);
  const closeWizard = useCallback(() => setWizardOpen(false), []);
  // Extras' tools: checked each time the tab opens, never before (egui `open_extras`).
  const extrasCheck = useToolCheck(client, native, { canInstall: native !== null && wizard.local, local: wizard.local });
  const recheckExtras = extrasCheck.recheck;
  useEffect(() => {
    if (route === "settings" && settingsTab === "extras") recheckExtras();
  }, [route, settingsTab, recheckExtras]);
  // General's Pull request style lists the PR styles that load, as they are on disk now.
  useEffect(() => {
    if (route === "settings" && settingsTab === "general") loadDefinitions();
  }, [route, settingsTab, loadDefinitions]);
  const [preferences, setPreferences] = usePreferences();
  const update = useUpdateCheck(native);
  const appVersion = useAppVersion(native);
  // A browser shows the daemon's version: the page is the daemon's build.
  const [daemonVersion, setDaemonVersion] = useState("");
  // egui's `set_view`: entering Settings reads the close choice again, as the close dialog may have remembered one.
  useEffect(() => {
    if (route !== "settings") return;
    if (native) {
      const asked = currentPreferences().closeChoice;
      native.readChoices().then(
        (c) => {
          // A chip clicked meanwhile is newer than this answer.
          if (currentPreferences().closeChoice !== asked) return;
          const closeChoice = c.closeChoice === "keep_running" || c.closeChoice === "stop_daemon" ? c.closeChoice : null;
          mergePreferences({ closeChoice });
        },
        () => {},
      );
    } else
      client.version().then(
        (v) => setDaemonVersion(v?.version ?? ""),
        () => {},
      );
  }, [route, native, client]);
  const [shortcutsPick, setShortcutsPick] = useState(FIRST_PICK);
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
    writeChoice("fleet.persona", persona, sayFailure);
  }, [personas.state, persona, savedPersona, sayFailure]);

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
  const archive = useArchive(client, say, reloadTasks);
  const daemonSettings = useDaemonSettings(native);
  const daemonRestart = useRestart(native, {
    loaded,
    agents: snapshot.agents,
    spawning: newTask.draft.busy,
    deciding: decidingRequest.size > 0,
    say,
    onRestarted: (reply) => {
      // Another address is another daemon: the page starts over against it, as after the version offer.
      const here = new URL(client.baseUrl || window.location.href, window.location.href).origin;
      if (new URL(reply.url).origin !== here) {
        void native?.setCloseArmed(false);
        carryNotice("success", "Daemon restarted");
        window.location.reload();
        return;
      }
      reloadSchema();
      reloadRepos();
      reloadPersonas();
      reloadProducts();
      reloadTasks();
    },
    onSettled: daemonSettings.settled,
  });
  const blocked = daemonRestart.daemonBlocker ?? newTaskBlocker(personas, products, savedProduct);
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

  // A draft lives only while its request does, as Lite's: a later request for the same repo starts afresh. A failed
  // decision leaves the request in place, so its draft is kept.
  useEffect(() => {
    const live = (id: string, forRepo: string) => snapshot.agents.some((a) => a.id === id && a.repo_request?.repo === forRepo);
    setRequests((r) => {
      const kept = Object.entries(r).filter(([id, d]) => live(id, d.forRepo));
      return kept.length === Object.keys(r).length ? r : Object.fromEntries(kept);
    });
  }, [snapshot.agents]);

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
          blocked={daemonRestart.daemonBlocker ?? (decidingRequest.has(open.id) ? "A decision is being sent" : null)}
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

  // As Lite's `set_active_product`: only the release filter belongs to a product.
  // egui's `set_active_product(None)`: ids are name slugs, so a product made later under the deleted one's name must
  // not come back active.
  const productDeleted = (id: string) => {
    if (id !== savedProduct) return;
    setSavedProduct(null);
    writeChoice("fleet.product", null, sayFailure);
  };
  const productsTab = useProductsTab(
    client,
    products,
    reloadProducts,
    reloadRepos,
    route === "management" && managementTab === "products",
    productDeleted,
  );
  // egui's New product modal, from the drawer and from Management → Products.
  const [newProductOpen, setNewProductOpen] = useState(false);
  const openNewProduct = () => setNewProductOpen(true);
  const closeNewProduct = useCallback(() => setNewProductOpen(false), []);
  const pickProduct = (id: string) => {
    setSavedProduct(id);
    writeChoice("fleet.product", id, say);
    setReleaseFilter({ type: "all" });
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
        writeChoice("fleet.persona", id, say);
      }}
      products={products}
      product={product}
      onProduct={pickProduct}
      newTaskBlocked={blocked}
      onNewTask={() => newTask.openDialog(spawnAs ?? "")}
      onNewProduct={openNewProduct}
      links={lastHref.current}
    />
  );
  const spawnAs = product === null ? null : spawnPersona(personas, products, product, persona);
  // As Lite's close_preview: the next card opens at the side, not where the last one left off.
  const closePanel = () => {
    setOpenId(null);
    panel.setChosenFull(false);
  };
  const footer = <Footer messages={messages} onClick={click} onSelect={setOpenId} onOpenLog={() => navigate(settingsHref("log"))} />;
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
      say={(text) => say("error", text)}
    />
  );
  const dialog =
    setup ||
    (newProductOpen && (
      <NewProduct
        client={client}
        personas={personas.state === "ready" ? personas.value : []}
        onCreated={(p) => {
          setNewProductOpen(false);
          pickProduct(p.id);
          reloadProducts();
        }}
        onClose={closeNewProduct}
        say={(text) => say("error", text)}
      />
    )) ||
    (managedSetup && (
      <PersonaSetup
        client={client}
        native={native}
        local={wizard.local}
        steps={wizard.steps}
        stillMissing={[]}
        draft={managedSetup.draft}
        mode={managedSetup.mode}
        onFinish={() => {
          setManagedSetup(null);
          reloadPersonas();
        }}
        onChanged={reloadPersonas}
        say={(text) => say("error", text)}
        onClose={() => {
          // A persona the setup made or checked may have changed since the list was read.
          setManagedSetup(null);
          reloadPersonas();
        }}
      />
    )) ||
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
        onWizard={() => {
          newTask.close();
          setWizardOpen(true);
        }}
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
        onRestart={() => daemonRestart.ask({ kind: "restart" })}
      />
      {wizardOpen && (
        <RelaunchedWizard
          client={client}
          native={native}
          local={wizard.local}
          daemon={daemonSettings}
          restart={daemonRestart}
          onClose={closeWizard}
        />
      )}
      {daemonRestart.dialog}
      <UpdateNotice native={native} blocked={!!main} />
    </>
  );

  if (route === "settings") {
    return (
      <Shell drawer={drawer} header={<h1 className="view-title">Settings</h1>} footer={footer} dialog={appDialogs(dialog)}>
        <SettingsView
          tab={settingsTab}
          body={(tab) =>
            tab === "general" ? (
              <General
                native={native}
                preferences={preferences}
                onPreferences={setPreferences}
                update={update}
                version={native ? appVersion : daemonVersion}
                say={say}
                daemon={daemonSettings}
                restart={daemonRestart}
                styles={definitions.st.list && usable(definitions.st.list, "pr_style")}
                onRelaunch={() => setWizardOpen(true)}
                shortcutsPick={shortcutsPick}
                onShortcutsPick={setShortcutsPick}
              />
            ) : tab === "log" ? (
              <LogTab inApp={native !== null} app={log.app} daemon={log.daemon} live={log.live} />
            ) : tab === "extras" ? (
              <Extras check={extrasCheck} native={native} onRelaunch={() => setWizardOpen(true)} />
            ) : tab === "daemon" ? (
              <DaemonTab native={native} restart={daemonRestart} />
            ) : (
              <NotYet />
            )
          }
        />
      </Shell>
    );
  }

  if (route === "management") {
    return (
      <Shell drawer={drawer} header={<h1 className="view-title">Management</h1>} footer={footer} dialog={appDialogs(dialog)}>
        <ManagementView
          tab={managementTab}
          body={(tab) => {
            switch (tab) {
              case "overview":
                return (
                  <Overview
                    products={products}
                    tasks={tasks?.tasks ?? []}
                    agents={snapshot.agents}
                    active={product}
                    onActive={pickProduct}
                    shown={overviewShown}
                    onShown={setOverviewShown}
                    now={now}
                  />
                );
              case "workflow":
                return <Workflow />;
              case "instructions": {
                const form = (schema && usableForm(schema, "instructions")) ?? null;
                return <Instructions config={agentConfig} form={form} />;
              }
              case "agent-settings":
                return <AgentSettings config={agentConfig} />;
              case "products":
                return (
                  <Products
                    products={products.state === "ready" ? products.value : null}
                    personas={personas.state === "ready" ? personas.value : []}
                    defs={definitions.st.list}
                    tab={productsTab}
                    onNewProduct={openNewProduct}
                    now={now}
                  />
                );
              case "personas":
                return (
                  <Personas
                    personas={personasTab}
                    onNew={() => setManagedSetup({ mode: "new", draft: null })}
                    onResume={(p) => setManagedSetup({ mode: "resume", draft: p })}
                    defs={definitions.st.list}
                    native={native}
                    setupOpen={managedSetup !== null}
                  />
                );
              case "orchestrators":
              case "specialists":
              case "skills":
              case "pr-styles":
                return <Definitions kind={DEF_TABS[tab]} defs={definitions} />;
              default:
                return <NotYet />;
            }
          }}
        />
      </Shell>
    );
  }

  if (route === "releases") {
    return (
      <Shell drawer={drawer} header={<h1 className="view-title">Releases</h1>} footer={footer} dialog={appDialogs(dialog)}>
        <ReleasesView
          client={client}
          product={product}
          releases={releases}
          tasks={tasks?.tasks ?? []}
          live={liveTasks(schema, snapshot.agents)}
          schema={schema}
          now={now}
          say={say}
          onChanged={reloadProducts}
          onTaskChanged={reloadTasks}
        />
      </Shell>
    );
  }

  if (route === "tasks") {
    const live = liveTasks(schema, snapshot.agents);
    return (
      <Shell
        drawer={drawer}
        header={
          <>
            <h1 className="view-title">Tasks</h1>
            <Button pressed={showArchived} onClick={() => setShowArchived((v) => !v)}>
              Show archived
            </Button>
            <div className="filters">
              <ReleaseFilter releases={releases} value={release} onChange={setReleaseFilter} />
            </div>
          </>
        }
        footer={footer}
        dialog={appDialogs(
          dialog || (archive.dialog && <ArchiveDialog dialog={archive.dialog} onGo={archive.go} onCancel={archive.cancel} />),
        )}
      >
        <TasksView
          load={taskLoad}
          onRetry={reloadTasks}
          live={live}
          personas={personaList}
          scope={{ product, release }}
          showArchived={showArchived}
          filter={tasksRoute.filter}
          task={tasksRoute.task}
          passport={(e, badge) => {
            const id = entryId(e);
            const agent = snapshot.agents.find((a) => a.id === id);
            const stop = schema && agent && stopAction(schema, agent);
            return (
              <Passport
                key={id}
                entry={e}
                badge={badge}
                client={client}
                now={now}
                live={live}
                product={productInfo}
                onArchive={() => archive.open({ kind: e.kind, id, name: entryName(e) })}
                onStop={() => stop && agent && void run(stop, agent)}
                onOpenOnBoard={() => {
                  navigate("#kanban");
                  setOpenId(id);
                }}
                onRefresh={refreshTasks}
                onChanged={reloadTasks}
                say={say}
              />
            );
          }}
        />
      </Shell>
    );
  }

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
              writeChoice("fleet.display", d, say);
            }}
          />
        </>
      }
      strip={<Strip tasks={tasks} stoppedTone={stoppedTone} selected={lastTasks.task} />}
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
