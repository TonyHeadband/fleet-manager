import { useState } from "react";
import { FleetError } from "../api/client";
import { initialValues, inputs, readyFor, submits, type FormValues } from "../api/forms";
import type { FormDef } from "../api/generated/FormDef";
import type { InputDef } from "../api/generated/InputDef";
import type { SubmitDef } from "../api/generated/SubmitDef";
import { Button } from "./Button";

/** What a choice input offers, by its source: data the page already has for the record the form is about. */
export interface Sources {
  product_repos?: string[];
}

interface Props {
  form: FormDef;
  /** `YYYY-MM-DD`, for a Today default. */
  today: string;
  initial?: FormValues;
  sources?: Sources;
  onSubmit: (submit: SubmitDef, values: FormValues) => Promise<void>;
  /** Shows a Cancel after the actions, as Lite's release form has. */
  onCancel?: () => void;
}

/** A form the daemon describes, one button per action. A refusal naming an input shows under it; any other under the form. */
export function Form({ form, today, initial, sources = {}, onSubmit, onCancel }: Props) {
  const [values, setValues] = useState<FormValues>(() => initial ?? initialValues(form, today));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field?: string; message: string } | null>(null);

  const run = async (submit: SubmitDef) => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit(submit, values);
    } catch (e) {
      setError(e instanceof FleetError ? { field: e.field, message: e.message } : { message: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const set = (key: string, value: string) => setValues({ ...values, [key]: value });

  const control = (def: InputDef) => {
    const value = values[def.key] ?? "";
    switch (def.kind) {
      case "multiline":
      case "markdown":
        return (
          <textarea
            className={def.kind === "markdown" ? "mono" : undefined}
            rows={def.kind === "markdown" ? 16 : 3}
            placeholder={def.placeholder ?? undefined}
            value={value}
            disabled={busy}
            onChange={(e) => set(def.key, e.target.value)}
          />
        );
      case "choice": {
        const options = def.source?.type === "product_repos" ? (sources.product_repos ?? []) : [];
        return (
          <select value={value} disabled={busy} onChange={(e) => set(def.key, e.target.value)}>
            <option value="" />
            {options.map((o) => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
        );
      }
      default:
        // A date is typed as YYYY-MM-DD, as in Lite: the daemon checks it and says why it refuses one.
        return (
          <input
            type="text"
            placeholder={def.placeholder ?? undefined}
            value={value}
            disabled={busy}
            onChange={(e) => set(def.key, e.target.value)}
          />
        );
    }
  };

  const input = (def: InputDef) => (
    <div className="form-input" data-key={def.key} key={def.key}>
      <label>
        {def.label}
        {control(def)}
      </label>
      {def.help && <p className="help">{def.help}</p>}
      {error?.field === def.key && (
        <p className="field-error" role="alert">
          {error.message}
        </p>
      )}
    </div>
  );

  const actions = submits(form);
  return (
    <form
      className="described-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (readyFor(form, actions[0]!, values)) void run(actions[0]!);
      }}
    >
      <h2>{form.title}</h2>
      {form.items.map((item, n) =>
        item.type === "group" ? (
          <div className="form-group" key={n}>
            {item.fields.map(input)}
          </div>
        ) : item.type === "field" ? (
          input(item)
        ) : null,
      )}
      {error && !inputHas(form, error.field) && <p role="alert">{error.message}</p>}
      <div className="form-actions">
        {actions.map((a, n) => (
          <Button key={a.id || n} type={n === 0 ? "submit" : "button"} disabled={busy || !readyFor(form, a, values)} onClick={n === 0 ? undefined : () => void run(a)}>
            {a.label}
          </Button>
        ))}
        {onCancel && (
          <Button className="ghost" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

function inputHas(form: FormDef, key: string | undefined) {
  return key !== undefined && inputs(form).some((i) => i.key === key);
}
