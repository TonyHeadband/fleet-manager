import { FilterMenu } from "../ui/FilterMenu";

interface Props {
  label: string;
  /** The row that clears the selection, and the summary while nothing is picked. */
  all: string;
  several: (n: number) => string;
  rows: { id: string; label: string; count: number }[];
  selected: string[];
  onChange: (selected: string[]) => void;
}

/**
 * The repo and persona filters' shared shape (Lite builds both alike): "All …" that only clears, a separator, then
 * a checkbox per row with its count at the right. Picks keep their click order.
 */
export function CheckFilter({ label, all, several, rows, selected, onChange }: Props) {
  const summary = selected.length === 0 ? all : selected.length === 1 ? (rows.find((r) => r.id === selected[0])?.label ?? selected[0]!) : several(selected.length);
  return (
    <FilterMenu label={label} summary={summary} name={`${label} filter`} width="menu">
      {() => (
        <>
          <div className="menu-row">
            <label>
              {/* Unticking "All" has nothing to fall back on, so the row only ever clears. */}
              <input type="checkbox" checked={selected.length === 0} onChange={() => selected.length > 0 && onChange([])} />
              {all}
            </label>
          </div>
          <hr />
          {rows.map((r) => {
            const on = selected.includes(r.id);
            return (
              <div key={r.id} className="menu-row">
                <label>
                  <input type="checkbox" checked={on} onChange={() => onChange(on ? selected.filter((s) => s !== r.id) : [...selected, r.id])} />
                  {r.label}
                </label>
                <span className="menu-count">{r.count}</span>
              </div>
            );
          })}
        </>
      )}
    </FilterMenu>
  );
}
