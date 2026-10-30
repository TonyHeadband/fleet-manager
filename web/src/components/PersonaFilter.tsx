import type { PersonaRow } from "../board/filters";
import { CheckFilter } from "./CheckFilter";

interface Props {
  rows: PersonaRow[];
  /** Persona ids, so a rename keeps the pick. Empty means every persona. */
  selected: string[];
  onChange: (selected: string[]) => void;
}

/** Lite's `persona_filter`: the board's filter, not the active persona. */
export function PersonaFilter({ rows, selected, onChange }: Props) {
  return <CheckFilter label="Persona" all="All personas" several={(n) => `${n} personas`} rows={rows} selected={selected} onChange={onChange} />;
}
