import type { ReactNode } from "react";
import { Button } from "./Button";

interface Props<Id extends string> {
  tabs: readonly { id: Id; label: string }[];
  selected: Id;
  onSelect: (id: Id) => void;
  /** Extra controls shown after the tabs; they are not tabs themselves. */
  children?: ReactNode;
}

export function Tabs<Id extends string>({ tabs, selected, onSelect, children }: Props<Id>) {
  return (
    <div role="tablist" className="tabs">
      {tabs.map((t) => (
        <Button key={t.id} role="tab" aria-selected={t.id === selected} onClick={() => onSelect(t.id)}>
          {t.label}
        </Button>
      ))}
      {children}
    </div>
  );
}
