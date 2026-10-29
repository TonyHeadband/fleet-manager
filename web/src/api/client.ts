import type { ActionDef } from "./generated/ActionDef";
import type { Agent } from "./generated/Agent";
import type { DiffReport } from "./generated/DiffReport";
import type { FormDef } from "./generated/FormDef";
import type { ProductList } from "./generated/ProductList";
import type { RepoList } from "./generated/RepoList";
import type { Snapshot } from "./generated/Snapshot";
import type { SpawnResponse } from "./generated/SpawnResponse";
import type { UiSchema } from "./generated/UiSchema";
import { formRequest, type FormValues } from "./forms";
import { actionRequest } from "./schema";

export class FleetError extends Error {
  /** The form input the daemon's refusal is about, when it names one. */
  constructor(
    message: string,
    readonly field?: string,
  ) {
    super(message);
  }
}

/** All calls go to one configured fleet endpoint; nothing assumes the daemon is on localhost. */
export class FleetClient {
  constructor(
    readonly baseUrl = "",
    private readonly fetchFn: typeof fetch = (input, init) => fetch(input, init),
  ) {}

  private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
    const init: RequestInit = { method };
    if (body !== undefined) {
      init.headers = { "content-type": "application/json" };
      init.body = JSON.stringify(body);
    }
    const res = await this.fetchFn(this.baseUrl + path, init);
    if (!res.ok) {
      const detail = (await res.json().catch(() => null)) as { error?: string; field?: string } | null;
      throw new FleetError(detail?.error ?? `HTTP ${res.status}`, detail?.field);
    }
    return (res.status === 204 ? undefined : await res.json()) as T;
  }

  snapshot = () => this.call<Snapshot>("GET", "/api/agents");
  uiSchema = () => this.call<UiSchema>("GET", "/api/ui-schema");
  products = () => this.call<ProductList>("GET", "/api/products");
  repos = async () => (await this.call<RepoList>("GET", "/api/repos")).repos;

  diff = (agentId: string) => this.call<DiffReport>("GET", `/api/agents/${encodeURIComponent(agentId)}/diff`);

  spawn = (repo: string, prompt: string) =>
    this.call<SpawnResponse>("POST", "/api/agents", { actor: "local", repos: [repo], prompt, slug: null });

  /** Sends a form the daemon described, to the path its description names. */
  submitForm = (form: FormDef, params: Record<string, string>, values: FormValues) => {
    const r = formRequest(form, params, values);
    return this.call<unknown>(r.method, r.path, r.body);
  };

  /** Runs any action from the daemon's UiSchema; the UI never names an action itself. */
  run = (action: ActionDef, agent: Agent, input?: string) => {
    const r = actionRequest(action, agent, input);
    return this.call<void>(r.method, r.path, r.body);
  };
}
