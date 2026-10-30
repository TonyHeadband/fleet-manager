import type { ReactNode } from "react";

/** egui's `checkbox`: the box, then its label; `title` is its hover text. */
export function Check(props: {
  checked: boolean;
  onChange: (on: boolean) => void;
  title?: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  const { checked, onChange, title, disabled, children } = props;
  return (
    <label className="check" title={title}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}
