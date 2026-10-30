import { CheckFilter } from "./CheckFilter";

interface Props {
  /** Each repo with the number of agents touching it, as Lite's repo filter lists them (`repoRows`). */
  repos: [string, number][];
  /** Empty means no filter: every task shows. */
  selected: string[];
  onChange: (selected: string[]) => void;
}

/** Lite's `repo_filter`: "All repos", one, or "N repos". */
export function RepoFilter({ repos, selected, onChange }: Props) {
  return (
    <CheckFilter
      label="Repo"
      all="All repos"
      several={(n) => `${n} repos`}
      rows={repos.map(([name, count]) => ({ id: name, label: name, count }))}
      selected={selected}
      onChange={onChange}
    />
  );
}
