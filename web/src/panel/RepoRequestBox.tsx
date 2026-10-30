import { useEffect, useId, useRef } from "react";
import type { FormDef } from "../api/generated/FormDef";
import type { RepoRequest } from "../api/generated/RepoRequest";
import type { SubmitDef } from "../api/generated/SubmitDef";
import { inputs, submits } from "../api/forms";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";

/** A repo request's draft, kept per agent by the caller: the repo to grant and the reason to refuse with. */
export interface RequestDraft {
  target: string;
  reason: string;
}

interface Props {
  request: RepoRequest;
  /** The agent's product's repos: the only ones a grant may name. */
  productRepos: string[];
  /** The daemon's `repo_request`, which names the actions and the captions. */
  form: FormDef;
  draft: RequestDraft;
  onDraft: (d: RequestDraft) => void;
  /** Why Grant and Refuse are off (a decision is on its way), or null. */
  blocked: string | null;
  /** Take the cursor in the reason box when it first draws (the panel just opened on this agent). */
  focus: boolean;
  onGrant: (repo: string) => void;
  onRefuse: (reason: string) => void;
}

/**
 * Lite's `repo_request_box`, in place of the reply box while an agent waits for a repo: what it wants and why, the
 * product's repos to grant from, and an optional reason for refusing. A repo the product lacks is added in Products
 * first; nothing here adds one. Neither action is sent from the keyboard: each is a deliberate press.
 */
export function RepoRequestBox({ request, productRepos, form, draft, onDraft, blocked, focus, onGrant, onRefuse }: Props) {
  const reasonBox = useRef<HTMLInputElement>(null);
  const reasonId = useId();
  useEffect(() => {
    if (focus) reasonBox.current?.focus();
    // Only on the first draw: a later focus would steal the cursor from wherever the user went.
  }, []);
  const input = (key: string) => inputs(form).find((i) => i.key === key);
  const action = (id: string) => submits(form).find((s) => s.id === id);
  const needsMet = (s: SubmitDef | undefined) =>
    !!s && s.needs.every((k) => (k === "repo" ? draft.target.trim() !== "" : k === "reason" ? draft.reason.trim() !== "" : true));
  const grant = action("grant");
  const refuse = action("refuse");
  const repoInput = input("repo");
  // The product's repos are the only options, so a target outside them is never granted.
  const canGrant = !blocked && productRepos.includes(draft.target) && needsMet(grant);
  const canRefuse = !blocked && needsMet(refuse);

  return (
    <div className="repo-request">
      <p className="request-wants">{`Wants ${request.repo}`}</p>
      {request.why !== "" && <p className="request-why">{request.why}</p>}
      {repoInput && (
        <div className="request-targets">
          <span className="mono-label">{repoInput.label}</span>
          {productRepos.map((r) => (
            <Chip key={r} selected={draft.target === r} onClick={() => onDraft({ ...draft, target: r })}>
              {r}
            </Chip>
          ))}
        </div>
      )}
      {!productRepos.includes(request.repo) && <p className="note">{`${request.repo} is not a repo of this product: add it in Products first, or refuse`}</p>}
      <div className="request-row">
        <Button variant="primary" disabled={!canGrant} title={blocked ?? undefined} onClick={() => onGrant(draft.target)}>
          {grant?.label ?? "Grant"}
        </Button>
      </div>
      <label className="mono-label" htmlFor={reasonId}>
        {input("reason")?.label ?? "Reason (optional)"}
      </label>
      <input id={reasonId} ref={reasonBox} type="text" value={draft.reason} onChange={(e) => onDraft({ ...draft, reason: e.target.value })} />
      <div className="request-row">
        <Button variant="quiet-danger" disabled={!canRefuse} title={blocked ?? undefined} onClick={() => onRefuse(draft.reason.trim())}>
          {refuse?.label ?? "Refuse"}
        </Button>
      </div>
    </div>
  );
}

/** This build's copy of the daemon's `repo_request`, used when the daemon's holds something it cannot draw. */
export const REPO_REQUEST: FormDef = (() => {
  const local = (id: string, label: string, path: string, needs: string[]): SubmitDef => ({
    id, label, method: "POST", path, fixed: { actor: "local" }, context: [], needs,
  });
  const field = (key: string, label: string, kind: "choice" | "text") => ({
    type: "field" as const, key, label, kind, placeholder: null, default: null, rules: [], help: null,
    source: kind === "choice" ? { type: "product_repos" as const } : null,
  });
  return {
    id: "repo_request",
    title: "Repo request",
    submit: local("grant", "Grant", "/api/agents/{id}/repo-request/grant", ["repo"]),
    items: [field("repo", "Grant access to", "choice"), field("reason", "Reason (optional)", "text")],
    more: [local("refuse", "Refuse", "/api/agents/{id}/repo-request/refuse", [])],
  };
})();
