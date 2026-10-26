import { useState } from "react";
import { Button } from "../ui/Button";

interface Props {
  repos: string[];
  /** Starts a task (one agent session) in `repo`. */
  onSpawn: (repo: string, prompt: string) => Promise<void>;
}

export function SpawnForm({ repos, onSpawn }: Props) {
  const [repo, setRepo] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chosen = repo || repos[0] || "";

  return (
    <form
      className="spawn"
      onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setError(null);
        try {
          await onSpawn(chosen, prompt);
          setPrompt("");
        } catch (err) {
          setError(err instanceof Error ? err.message : String(err));
        } finally {
          setBusy(false);
        }
      }}
    >
      <label>
        Repo
        <select value={chosen} onChange={(e) => setRepo(e.target.value)}>
          {repos.map((p) => (
            <option key={p}>{p}</option>
          ))}
        </select>
      </label>
      <label className="grow">
        Prompt
        <textarea rows={2} value={prompt} onChange={(e) => setPrompt(e.target.value)} />
      </label>
      <Button type="submit" disabled={busy || !chosen || !prompt.trim()}>
        Start agent
      </Button>
      {error && <p role="alert">{error}</p>}
    </form>
  );
}
