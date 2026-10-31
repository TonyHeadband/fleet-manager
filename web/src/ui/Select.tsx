import { FilterMenu } from "./FilterMenu";

interface Props {
  /** For the field's `<label htmlFor>`, which names it. */
  id?: string;
  /** Its name where no `<label>` names it. */
  label?: string;
  value: string;
  options: readonly { value: string; label: string }[];
  /** Shown while the value is none of the options; without it, the value itself shows (a raw path). */
  placeholder?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}

/** A form's single choice in the app's own selector, as wide as its field: a combobox opening a listbox of options. */
export function Select({ id, label, value, options, placeholder, disabled, onChange }: Props) {
  const summary = options.find((o) => o.value === value)?.label ?? placeholder ?? value;
  return (
    <div className="select">
      <FilterMenu id={id} name={label} summary={summary} width="fill" disabled={disabled} listbox>
        {(close) =>
          options.map((o) => (
            <button
              key={o.value}
              type="button"
              role="option"
              className="menu-item"
              aria-selected={o.value === value}
              onClick={() => {
                close();
                if (o.value !== value) onChange(o.value);
              }}
            >
              {o.label}
            </button>
          ))
        }
      </FilterMenu>
    </div>
  );
}
