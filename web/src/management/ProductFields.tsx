import { useId } from "react";
import type { EnvLine } from "../api/generated/EnvLine";
import type { Persona } from "../api/generated/Persona";
import type { RepoInfo } from "../api/generated/RepoInfo";
import { workflowLine } from "../newTask/model";
import { CaptionField } from "../personas/logins";
import { Button } from "../ui/Button";
import { ErrorLine } from "../ui/ErrorLine";
import { blank, newRepo, valid, type ProductForm, type RepoDraft } from "./productForm";
import { Select } from "../ui/Select";

// A port of product_fields in crates/fleet-gui/src/components/product_form.rs, with the Products tab's lines under a
// stored repo row (products.rs's `repo_extra`).

interface Props {
  form: ProductForm;
  onChange: (form: ProductForm) => void;
  /** The listed personas; the default is picked among the ready ones. */
  personas: Persona[];
  /** The stored repo of that name, for its workflow and env lines; none in New product. */
  repoInfo: (name: string) => RepoInfo | undefined;
  /** The daemon's refusals, by repo name. */
  repoErrors: Record<string, string>;
}

const FIELDS = [
  ["name", "Repo name"],
  ["path", "Local path"],
  ["url", "Clone URL"],
] as const;

export function ProductFields({ form, onChange, personas, repoInfo, repoErrors }: Props) {
  const personaId = useId();
  const ready = personas.filter((p) => p.ready);
  const set = (change: Partial<ProductForm>) => onChange({ ...form, ...change });
  const setRepo = (i: number, change: Partial<RepoDraft>) => set({ repos: form.repos.map((r, n) => (n === i ? { ...r, ...change } : r)) });
  return (
    <div className="product-fields">
      <CaptionField label="Name" value={form.name} hint="What the product is called" onChange={(name) => set({ name })} />
      <div className="caption-field">
        <label className="mono-label caption" htmlFor={personaId}>
          Default persona
        </label>
        {/* A stored default that is not a ready persona shows as No default, and is kept unless changed. */}
        <Select
          id={personaId}
          value={ready.some((p) => p.id === form.defaultPersona) ? (form.defaultPersona ?? "") : ""}
          options={[{ value: "", label: "No default" }, ...ready.map((p) => ({ value: p.id, label: p.name }))]}
          onChange={(v) => set({ defaultPersona: v === "" ? null : v })}
        />
      </div>
      <div>
        <CaptionField
          label="Internal repo URL"
          value={form.internalUrl}
          hint="https://github.com/org/notes.git"
          onChange={(internalUrl) => set({ internalUrl })}
        />
        <p className="note">HTTPS URL. Cloned with the default persona's GitHub login; tasks push their notes here.</p>
      </div>
      <div className="product-repos">
        <span className="mono-label caption">Repos</span>
        {form.repos.map((repo, i) => {
          const stored = repo.existing ? repoInfo(repo.name.trim()) : undefined;
          const error = repoErrors[repo.name.trim()];
          return (
            <div key={i} className="product-repo">
              <div className="product-repo-row">
                {FIELDS.map(([key, hint]) => {
                  const fixed = (key === "name" && repo.existing) || (key === "url" && repo.hadUrl);
                  return (
                    <input
                      key={key}
                      aria-label={`${hint} ${i + 1}`}
                      placeholder={hint}
                      value={repo[key]}
                      readOnly={fixed}
                      title={key === "url" && fixed ? "Remove the repo and add it again to change its URL" : undefined}
                      spellCheck={false}
                      onChange={(e) => setRepo(i, { [key]: e.target.value })}
                    />
                  );
                })}
                <Button
                  className="ghost"
                  aria-label={`Remove repo ${i + 1}`}
                  onClick={() => set({ repos: form.repos.filter((_, n) => n !== i) })}
                >
                  Remove
                </Button>
              </div>
              {repo.hadPath && repo.path.trim() === "" && <p className="note">A path cannot be cleared; Revert restores it.</p>}
              {error !== undefined && <ErrorLine text={error} />}
              {stored && <RepoLines repo={stored} />}
            </div>
          );
        })}
        <div>
          <Button onClick={() => set({ repos: [...form.repos, newRepo()] })}>Add repo</Button>
        </div>
        {form.repos.some((r) => !blank(r) && !valid(r)) && <p className="note">A repo needs a name and a local path or a clone URL.</p>}
      </div>
    </div>
  );
}

const envText = (e: EnvLine) =>
  `${e.name} = ${e.value} · ${e.source === "repo" ? "repo" : e.overridden ? "fleet, overridden by the repo" : "fleet, not in the gate"}`;

/** Under a stored repo: what its workflow file says, then what its tasks run with. */
function RepoLines({ repo }: { repo: RepoInfo }) {
  const line = workflowLine(repo.name, repo.workflow);
  return (
    <div className="repo-lines">
      {line.blocks ? <ErrorLine text={line.text} /> : <p className="repo-line">{line.text}</p>}
      {repo.env.map((e) => (
        <p key={`${e.source}:${e.name}`} className="repo-line">
          {envText(e)}
        </p>
      ))}
    </div>
  );
}
