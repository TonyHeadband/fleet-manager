import type { ActionDef } from "./generated/ActionDef";
import type { EffectiveSettings } from "./generated/EffectiveSettings";
import type { InternalBranches } from "./generated/InternalBranches";
import type { InternalFile } from "./generated/InternalFile";
import type { InternalFiles } from "./generated/InternalFiles";
import type { InternalHistory } from "./generated/InternalHistory";
import type { NewProduct } from "./generated/NewProduct";
import type { Product } from "./generated/Product";
import type { ProductOverride } from "./generated/ProductOverride";
import type { ProductUpdate } from "./generated/ProductUpdate";
import type { RepoInfo } from "./generated/RepoInfo";
import type { RepoUpsert } from "./generated/RepoUpsert";
import type { Agent } from "./generated/Agent";
import type { AgentConfig } from "./generated/AgentConfig";
import type { AgentSettings } from "./generated/AgentSettings";
import type { AskAnswer } from "./generated/AskAnswer";
import type { ChatPage } from "./generated/ChatPage";
import type { Conversation } from "./generated/Conversation";
import type { CheckoutList } from "./generated/CheckoutList";
import type { DefFile } from "./generated/DefFile";
import type { DefInfo } from "./generated/DefInfo";
import type { DefKind } from "./generated/DefKind";
import type { DiffReport } from "./generated/DiffReport";
import type { DecideBreakdown } from "./generated/DecideBreakdown";
import type { FleetView } from "./generated/FleetView";
import type { FormDef } from "./generated/FormDef";
import type { Release } from "./generated/Release";
import type { NewPersona } from "./generated/NewPersona";
import type { Persona } from "./generated/Persona";
import type { PersonaList } from "./generated/PersonaList";
import type { PersonaUpdate } from "./generated/PersonaUpdate";
import type { ProductList } from "./generated/ProductList";
import type { SubmitDef } from "./generated/SubmitDef";
import type { RepoList } from "./generated/RepoList";
import type { Snapshot } from "./generated/Snapshot";
import type { SpawnRequest } from "./generated/SpawnRequest";
import type { StopTaskResponse } from "./generated/StopTaskResponse";
import type { SpawnResponse } from "./generated/SpawnResponse";
import type { Task } from "./generated/Task";
import type { TaskList } from "./generated/TaskList";
import type { ArchivePlan } from "./generated/ArchivePlan";
import type { ArchiveReport } from "./generated/ArchiveReport";
import type { ToolError } from "./generated/ToolError";
import type { ToolList } from "./generated/ToolList";
import type { UiSchema } from "./generated/UiSchema";
import type { VersionInfo } from "./generated/VersionInfo";
import { formRequest, type FormValues } from "./forms";
import { actionRequest } from "./schema";

/** fleet-api's `PLAN_CHANGED`: an archive refused because the task changed since its plan was shown. */
export const PLAN_CHANGED = "the task changed since you confirmed; review the new plan";

export class FleetError extends Error {
  /** The form input the daemon's refusal is about, when it names one; the tool a spawn was refused for, with its fix. */
  constructor(
    message: string,
    readonly field?: string,
    readonly tool?: ToolError,
    /** The HTTP status of a refusal. */
    readonly status?: number,
  ) {
    super(message);
  }
}

/** fleet-client's `TIMEOUT`: a request the daemon took and never answers fails, so the list it was for is asked for
 * again instead of waiting until the connection dies. The event streams are not calls and never time out. */
const TIMEOUT_MS = 10_000;
/** fleet-client's `CHECK_TIMEOUT`: the daemon runs a persona's `claude` and `gh` checks first. */
const CHECK_TIMEOUT_MS = 25_000;
/** fleet-client's `TOOLS_TIMEOUT`: the daemon may run every tool's probe before it answers. A check with no answer
 * by then fails, and startup moves on as egui does. */
const TOOLS_TIMEOUT_MS = 45_000;
/** fleet-client's `LONG_TIMEOUT`, for calls that may clone a repo or weigh and remove folders: the daemon finishes the
 * work whether or not the client waits, so giving up early only hides the result. */
const LONG_TIMEOUT_MS = 30 * 60_000;

const productPath = (id: string) => `/api/products/${encodeURIComponent(id)}`;
const internalQuery = (branch: string, path: string) => `branch=${encodeURIComponent(branch)}&path=${encodeURIComponent(path)}`;

const defPath = (kind: DefKind, name: string) => `/api/definitions/${kind}/${encodeURIComponent(name)}`;

/** A body sent as it is, not as JSON: a definition file. */
class PlainText {
  constructor(readonly text: string) {}
}

/** `url` with the app's token as a query parameter: a stream's, which EventSource opens without headers of its own. */
export const withToken = (url: string, token: string | null | undefined) =>
  token ? `${url}${url.includes("?") ? "&" : "?"}token=${encodeURIComponent(token)}` : url;

/** All calls go to one configured fleet endpoint; nothing assumes the daemon is on localhost. */
export class FleetClient {
  /** The Tauri app's key to its daemon, sent as `X-Fleet-Token`; null in a browser and with an older daemon. Set by
   * `takeDaemonToken` when the app connects and when the daemon changes. */
  token: string | null = null;

  constructor(
    readonly baseUrl = "",
    private readonly fetchFn: typeof fetch = (input, init) => fetch(input, init),
  ) {}

  private async call<T>(method: string, path: string, body?: unknown, timeoutMs = TIMEOUT_MS): Promise<T> {
    const init: RequestInit = { method };
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(new FleetError("the daemon did not answer in time")), timeoutMs);
    init.signal = abort.signal;
    const headers: Record<string, string> = {};
    if (body instanceof PlainText) {
      headers["content-type"] = "text/plain";
      init.body = body.text;
    } else if (body !== undefined) {
      headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    if (this.token) headers["x-fleet-token"] = this.token;
    if (Object.keys(headers).length > 0) init.headers = headers;
    const res = await this.fetchFn(this.baseUrl + path, init).finally(() => clearTimeout(timer));
    if (!res.ok) {
      const detail = (await res.json().catch(() => null)) as { error?: string; field?: string; tool?: ToolError } | null;
      throw new FleetError(detail?.error ?? `HTTP ${res.status}`, detail?.field, detail?.tool, res.status);
    }
    return (res.status === 204 ? undefined : await res.json()) as T;
  }

  snapshot = () => this.call<Snapshot>("GET", "/api/agents");
  uiSchema = () => this.call<UiSchema>("GET", "/api/ui-schema");
  /** The daemon's version; null for an older daemon without the route. */
  version = () =>
    this.call<VersionInfo>("GET", "/api/version").catch((e: unknown) => {
      if (e instanceof FleetError && e.status === 404) return null;
      throw e;
    });
  /** The tools the daemon checked for, and why any cannot be used. */
  tools = () => this.call<ToolList>("GET", "/api/tools", undefined, TOOLS_TIMEOUT_MS);
  /** Asks the daemon to look for the tools again, and answers with what it found. */
  recheckTools = () => this.call<ToolList>("POST", "/api/tools/recheck", undefined, TOOLS_TIMEOUT_MS);
  /** The page of an agent's chat that ends just before `before`. */
  chatHistory = (id: string, before: number, limit: number) =>
    this.call<ChatPage>("GET", `/api/agents/${encodeURIComponent(id)}/chat/history?before=${before}&limit=${limit}`);
  /** Rejects with the daemon's reason, e.g. when the ask was already answered. */
  answerAsk = (id: string, ask: string, answer: AskAnswer) =>
    this.call<void>("POST", `/api/agents/${encodeURIComponent(id)}/asks/${encodeURIComponent(ask)}`, answer);
  products = () => this.call<ProductList>("GET", "/api/products");
  createProduct = (p: NewProduct) => this.call<Product>("POST", "/api/products", p, LONG_TIMEOUT_MS);
  /** An absent field is left alone; `null` clears the default persona or the internal URL. */
  updateProduct = (id: string, u: Partial<ProductUpdate>) => this.call<Product>("PATCH", productPath(id), u, LONG_TIMEOUT_MS);
  /** Adds a repo, or fills in the path or URL it lacks. */
  upsertRepo = (id: string, r: RepoUpsert) => this.call<RepoInfo>("PUT", `${productPath(id)}/repos`, r);
  deleteRepo = (id: string, name: string) => this.call<void>("DELETE", `${productPath(id)}/repos/${encodeURIComponent(name)}`);
  deleteProduct = (id: string) => this.call<void>("DELETE", productPath(id));
  putHiddenSpecialists = (id: string, hidden: string[]) => this.call<Product>("PUT", `${productPath(id)}/hidden-specialists`, { hidden });
  /** The whole override list: it replaces what is stored. */
  putOverrides = (id: string, overrides: ProductOverride[]) => this.call<Product>("PUT", `${productPath(id)}/overrides`, { overrides });
  /** The global settings as agents get them, each with where it comes from. */
  effectiveSettings = () => this.call<EffectiveSettings>("GET", "/api/settings/effective");
  internalBranches = (id: string) => this.call<InternalBranches>("GET", `${productPath(id)}/internal/branches`);
  internalFiles = async (id: string, branch: string) =>
    (await this.call<InternalFiles>("GET", `${productPath(id)}/internal/files?branch=${encodeURIComponent(branch)}`)).files;
  internalFile = async (id: string, branch: string, path: string) =>
    (await this.call<InternalFile>("GET", `${productPath(id)}/internal/file?${internalQuery(branch, path)}`)).text;
  /** The commits of one file, newest first. */
  internalHistory = async (id: string, branch: string, path: string) =>
    (await this.call<InternalHistory>("GET", `${productPath(id)}/internal/history?${internalQuery(branch, path)}`)).commits;
  agentConfig = () => this.call<AgentConfig>("GET", "/api/agent-config");
  /** Saves the instructions, the settings, or both; what is left out stays as it is. */
  saveAgentConfig = (u: { instructions?: string; settings?: AgentSettings }) => this.call<AgentConfig>("PUT", "/api/agent-config", u);
  /** The repos of one product, or of every product. */
  repos = async (product?: string) =>
    (await this.call<RepoList>("GET", product === undefined ? "/api/repos" : `/api/repos?product=${encodeURIComponent(product)}`)).repos;
  /** The live checkouts of a product repo's local folder on the daemon's machine, main first. */
  checkouts = async (product: string, name: string) =>
    (await this.call<CheckoutList>("GET", `/api/products/${encodeURIComponent(product)}/repos/${encodeURIComponent(name)}/checkouts`))
      .checkouts;
  /** The orchestrator, specialist, skill and PR style definitions, with why any cannot be used. */
  definitions = () => this.call<DefInfo[]>("GET", "/api/definitions");
  definition = (kind: DefKind, name: string) => this.call<DefFile>("GET", defPath(kind, name));
  /** Saves `text` as the user's file, overriding a built-in of the same name; the daemon refuses a file it cannot load. */
  saveDefinition = (kind: DefKind, name: string, text: string) =>
    this.call<DefInfo>("PUT", defPath(kind, name), new PlainText(text));
  /** Deletes the user's copy, so the built-in applies again. */
  resetDefinition = (kind: DefKind, name: string) => this.call<void>("DELETE", defPath(kind, name));

  diff = (agentId: string) => this.call<DiffReport>("GET", `/api/agents/${encodeURIComponent(agentId)}/diff`);

  personas = () => this.call<PersonaList>("GET", "/api/personas");
  createPersona = (p: NewPersona) => this.call<Persona>("POST", "/api/personas", p);
  /** Asks the daemon to check the persona's logins now, and answers with the persona as found. */
  checkPersona = (id: string) => this.call<Persona>("POST", `/api/personas/${encodeURIComponent(id)}/check`, undefined, CHECK_TIMEOUT_MS);
  deletePersona = (id: string) => this.call<void>("DELETE", `/api/personas/${encodeURIComponent(id)}`);
  /** Changes only the fields set; the others are null. */
  updatePersona = (id: string, u: PersonaUpdate) => this.call<Persona>("PATCH", `/api/personas/${encodeURIComponent(id)}`, u);
  tasks = () => this.call<TaskList>("GET", "/api/tasks");
  /** Asks the daemon to measure the tasks' footprints again. */
  refreshTasks = () => this.call<void>("POST", "/api/tasks/refresh");
  archivePlan = (id: string) =>
    this.call<ArchivePlan>("GET", `/api/tasks/${encodeURIComponent(id)}/archive-plan`, undefined, LONG_TIMEOUT_MS);
  /** Archives with the plan the person saw; the daemon refuses with `PLAN_CHANGED` when the task moved on since. */
  archive = (id: string, plan: ArchivePlan) =>
    this.call<ArchiveReport>("POST", `/api/tasks/${encodeURIComponent(id)}/archive`, { items: plan.items }, LONG_TIMEOUT_MS);
  orphanPlan = (id: string) => this.call<ArchivePlan>("POST", "/api/orphans/plan", { id }, LONG_TIMEOUT_MS);
  archiveOrphan = (id: string, plan: ArchivePlan) =>
    this.call<ArchiveReport>("POST", "/api/orphans/archive", { id, items: plan.items }, LONG_TIMEOUT_MS);
  /** Marks a task with no GitHub repo done; the daemon refuses one with a repo. */
  markDone = (task: string) => this.call<Task>("POST", `/api/tasks/${encodeURIComponent(task)}/done`);
  /** Moves a task into a release of its product, or to the backlog with `null`. */
  setTaskRelease = (task: string, release: number | null) =>
    this.call<Task>("PUT", `/api/tasks/${encodeURIComponent(task)}/release`, { release });

  spawn = (req: SpawnRequest) => this.call<SpawnResponse>("POST", "/api/agents", req, LONG_TIMEOUT_MS);

  /** The persona's supervisor conversations, most recently used first. */
  conversations = (persona: string) => this.call<Conversation[]>("GET", `/api/supervisor/${encodeURIComponent(persona)}/conversations`);
  /** Starts a conversation; the daemon checks the persona first. */
  newConversation = (persona: string, text: string) =>
    this.call<Conversation>("POST", "/api/supervisor/conversations", { persona, text }, CHECK_TIMEOUT_MS);
  /** The daemon may resume the conversation's process first and waits for it to be ready. It delivers the message
   * whether or not the client waits, so giving up at the check timeout would only invite a duplicate resend. */
  sendToConversation = (id: string, text: string) =>
    this.call<void>("POST", `/api/supervisor/conversations/${encodeURIComponent(id)}/message`, { text }, LONG_TIMEOUT_MS);

  /** An orchestrated task's breakdowns and gate runs. */
  fleetView = (taskId: string) => this.call<FleetView>("GET", `/api/tasks/${encodeURIComponent(taskId)}/fleet`);
  /** Gives an agent waiting for a repo access to one of its product's repos. */
  grantRepo = (agentId: string, repo: string) =>
    this.call<unknown>("POST", `/api/agents/${encodeURIComponent(agentId)}/repo-request/grant`, { actor: "local", repo }, LONG_TIMEOUT_MS);
  refuseRepo = (agentId: string, reason: string) =>
    this.call<void>("POST", `/api/agents/${encodeURIComponent(agentId)}/repo-request/refuse`, { actor: "local", reason });
  /** Approve, or ask for changes to, the breakdown version the owner saw. */
  decideBreakdown = (taskId: string, d: DecideBreakdown) =>
    this.call<void>("POST", `/api/tasks/${encodeURIComponent(taskId)}/breakdown/decide`, d);
  /** Stops every agent of an orchestrated task. */
  stopTask = (taskId: string) => this.call<StopTaskResponse>("POST", `/api/tasks/${encodeURIComponent(taskId)}/stop`);
  /** Starts a new orchestrator for a task whose orchestrator has ended. */
  restartOrchestrator = (taskId: string) =>
    this.call<Agent>("POST", `/api/tasks/${encodeURIComponent(taskId)}/restart`, undefined, CHECK_TIMEOUT_MS);

  /** Edits a release; `released` marks it released or not. The create and edit forms go through `submitForm`. */
  updateRelease = (id: number, u: { version?: string; start?: string; target?: string; released?: boolean }) =>
    this.call<Release>("PATCH", `/api/releases/${id}`, u);
  /** The release's tasks go back to the backlog. */
  deleteRelease = (id: number) => this.call<void>("DELETE", `/api/releases/${id}`);

  /** Sends a form the daemon described, to the path its description names. */
  submitForm = (
    form: FormDef,
    submit: SubmitDef,
    params: Record<string, string>,
    values: FormValues,
    context?: Record<string, unknown>,
  ) => {
    const r = formRequest(form, submit, params, values, context);
    return this.call<unknown>(r.method, r.path, r.body);
  };

  /** Runs any action from the daemon's UiSchema; the UI never names an action itself. */
  run = (action: ActionDef, agent: Agent, input?: string) => {
    const r = actionRequest(action, agent, input);
    return this.call<void>(r.method, r.path, r.body);
  };
}
