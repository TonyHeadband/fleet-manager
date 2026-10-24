import { Button } from "../ui/Button";

interface Props {
  repos: string[];
  /** Empty means no filter: every agent shows. */
  selected: string[];
  onChange: (selected: string[]) => void;
}

export function RepoFilter({ repos, selected, onChange }: Props) {
  return (
    <div className="filter" role="group" aria-label="Repo filter">
      {repos.map((p) => {
        const on = selected.includes(p);
        return (
          <Button key={p} pressed={on} onClick={() => onChange(on ? selected.filter((s) => s !== p) : [...selected, p])}>
            {p}
          </Button>
        );
      })}
    </div>
  );
}
