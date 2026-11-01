import type { ReactNode } from "react";
import { FilterMenu } from "./FilterMenu";

interface Props<T extends string> {
  label: string;
  name: string;
  width: "menu" | "release" | "fill";
  items: { id: T; name: string }[];
  value: T | null;
  /** The summary while nothing is picked. */
  placeholder?: string;
  onPick: (id: T) => void;
  lead?: ReactNode;
  captionAt?: "side" | "above";
}

/** The filter combo as a single choice: the current pick as its summary, and a menu that closes on a pick. */
export function ChoiceMenu<T extends string>({ label, name, width, items, value, placeholder = "", onPick, lead, captionAt }: Props<T>) {
  return (
    <FilterMenu label={label} summary={items.find((i) => i.id === value)?.name ?? placeholder} name={name} width={width} lead={lead} captionAt={captionAt}>
      {(close) =>
        items.map((i) => (
          <button
            key={i.id}
            type="button"
            className="menu-item"
            aria-pressed={i.id === value}
            onClick={() => {
              close();
              if (i.id !== value) onPick(i.id);
            }}
          >
            {i.name}
          </button>
        ))
      }
    </FilterMenu>
  );
}
