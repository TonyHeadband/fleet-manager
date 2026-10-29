import { useEffect, useState } from "react";
import type { RepoInfo } from "../api/generated/RepoInfo";
import type { SpawnRequest } from "../api/generated/SpawnRequest";
import { spawnRequest } from "../shell/choices";
import { Button } from "../ui/Button";

interface Props {
  product: string;
  persona: string;
  /** The active product's repos. */
  repos: RepoInfo[];
  onSpawn: (req: SpawnRequest) => Promise<void>;
  onClose: () => void;
}

/**
 * New task, opened from the drawer: the goal and the product's repos to start with, in Lite's words. Each repo starts
 * from its URL; one with only a local path waits for the full dialog, which needs Lite's lookup of its main checkout.
 */
export function SpawnForm({ product, persona, repos, onSpawn, onClose }: Props) {
  const [chosen, setChosen] = useState<string[]>([]);
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ready = !busy && goal.trim() !== "";

  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  const start = async () => {
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      await onSpawn(spawnRequest(product, persona, chosen, goal));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop">
      <form
        className="dialog spawn"
        role="dialog"
        aria-modal="true"
        aria-label="New task"
        onSubmit={(e) => {
          e.preventDefault();
          void start();
        }}
      >
        <h2>New task</h2>
        <label className="grow">
          Goal
          <textarea
            rows={4}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            onKeyDown={(e) => {
              // Ctrl+Enter sends, as in Lite's dialog.
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                void start();
              }
            }}
          />
        </label>
        <fieldset>
          <legend>Repos (optional)</legend>
          {repos.map((r) => {
            const localOnly = r.url === null;
            return (
              <label key={r.name} title={localOnly ? "start it from Lite for now" : undefined}>
                <input
                  type="checkbox"
                  disabled={localOnly}
                  checked={chosen.includes(r.name)}
                  onChange={(e) => setChosen(e.target.checked ? [...chosen, r.name] : chosen.filter((c) => c !== r.name))}
                />
                {r.name}
              </label>
            );
          })}
        </fieldset>
        {error && <p role="alert">{error}</p>}
        <div className="form-actions">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            Start task
          </Button>
        </div>
      </form>
    </div>
  );
}
