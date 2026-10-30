import { Button } from "./Button";
import { SmallIconButton } from "./SmallIconButton";

const replace = <T,>(rows: T[], i: number, row: T) => rows.map((r, j) => (j === i ? row : r));
const without = <T,>(rows: T[], i: number) => rows.filter((_, j) => j !== i);

/** One field per row, named "{noun} {n}", each with its remove button, then "Add {noun}" (egui's `rules`). */
export function RowsEditor({
  rows,
  noun,
  hint,
  onChange,
}: {
  rows: string[];
  noun: string;
  hint: string;
  onChange: (rows: string[]) => void;
}) {
  return (
    <div className="rows-editor">
      {rows.map((row, i) => (
        <div className="rows-editor-row" key={i}>
          <input
            className="mono"
            aria-label={`${noun} ${i + 1}`}
            placeholder={hint}
            value={row}
            onChange={(e) => onChange(replace(rows, i, e.target.value))}
          />
          <SmallIconButton icon="close" label={`Remove ${noun} ${i + 1}`} onClick={() => onChange(without(rows, i))} />
        </div>
      ))}
      <div>
        <Button onClick={() => onChange([...rows, ""])}>{`Add ${noun}`}</Button>
      </div>
    </div>
  );
}

/** A name and a value per row, as egui's environment rows. */
export function PairRowsEditor({ rows, onChange }: { rows: [string, string][]; onChange: (rows: [string, string][]) => void }) {
  return (
    <div className="rows-editor">
      {rows.map(([key, value], i) => (
        <div className="rows-editor-row" data-pair key={i}>
          <input
            className="mono"
            aria-label={`Variable name ${i + 1}`}
            placeholder="NAME"
            value={key}
            onChange={(e) => onChange(replace(rows, i, [e.target.value, value]))}
          />
          <input
            className="mono"
            aria-label={`Variable value ${i + 1}`}
            placeholder="value"
            value={value}
            onChange={(e) => onChange(replace(rows, i, [key, e.target.value]))}
          />
          <SmallIconButton icon="close" label={`Remove variable ${i + 1}`} onClick={() => onChange(without(rows, i))} />
        </div>
      ))}
      <div>
        <Button onClick={() => onChange([...rows, ["", ""]])}>Add variable</Button>
      </div>
    </div>
  );
}
