import type { Release } from "../api/generated/Release";
import { offered, type ReleaseFilter as Filter } from "../board/filters";
import { FilterMenu } from "../ui/FilterMenu";

interface Props {
  /** The active product's releases; only unreleased ones are offered. */
  releases: Release[];
  value: Filter;
  onChange: (f: Filter) => void;
}

const same = (a: Filter, b: Filter) => a.type === b.type && (a.type !== "release" || (b.type === "release" && a.id === b.id));

/** Lite's `release_filter`: one choice of All releases, an unreleased version, or the Backlog; it closes on a pick. */
export function ReleaseFilter({ releases, value, onChange }: Props) {
  const rows: [Filter, string][] = [[{ type: "all" }, "All releases"], ...offered(releases).map((r): [Filter, string] => [{ type: "release", id: r.id }, r.version]), [{ type: "backlog" }, "Backlog"]];
  const summary = value.type === "all" ? "All releases" : value.type === "backlog" ? "Backlog" : (releases.find((r) => r.id === value.id)?.version ?? "Release");
  return (
    <FilterMenu label="Release" summary={summary} name="Release filter" width="release">
      {(close) =>
        rows.map(([f, text]) => (
          <button
            key={text}
            type="button"
            className="menu-item"
            aria-pressed={same(f, value)}
            onClick={() => {
              close();
              if (!same(f, value)) onChange(f);
            }}
          >
            {text}
          </button>
        ))
      }
    </FilterMenu>
  );
}
