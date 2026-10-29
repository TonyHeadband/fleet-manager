import { Button } from "../ui/Button";

interface Props {
  /** Each repo with the number of agents on the board touching it, as Lite's repo filter lists them. */
  repos: [string, number][];
  /** Empty means no filter: every agent shows. */
  selected: string[];
  onChange: (selected: string[]) => void;
}

export function RepoFilter({ repos, selected, onChange }: Props) {
  return (
    <div className="filter" role="group" aria-label="Repo filter">
      {repos.map(([p, count]) => {
        const on = selected.includes(p);
        return (
          <Button key={p} pressed={on} onClick={() => onChange(on ? selected.filter((s) => s !== p) : [...selected, p])}>
            {p} <span className="count">{count}</span>
          </Button>
        );
      })}
    </div>
  );
}
