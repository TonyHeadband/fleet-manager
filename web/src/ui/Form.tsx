import { useState } from "react";
import { FleetError } from "../api/client";
import { initialValues, inputs, ready, type FormValues } from "../api/forms";
import type { FormDef } from "../api/generated/FormDef";
import type { InputDef } from "../api/generated/InputDef";
import { Button } from "./Button";

interface Props {
  form: FormDef;
  /** `YYYY-MM-DD`, for a Today default. */
  today: string;
  initial?: FormValues;
  onSubmit: (values: FormValues) => Promise<void>;
}

/** A form the daemon describes. A refusal naming an input shows under it; any other under the form. */
export function Form({ form, today, initial, onSubmit }: Props) {
  const [values, setValues] = useState<FormValues>(() => initial ?? initialValues(form, today));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field?: string; message: string } | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit(values);
    } catch (e) {
      setError(e instanceof FleetError ? { field: e.field, message: e.message } : { message: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const input = (def: InputDef) => (
    <div className="form-input" data-key={def.key} key={def.key}>
      <label>
        {def.label}
        <input
          type={def.kind === "date" ? "date" : "text"}
          placeholder={def.placeholder ?? undefined}
          value={values[def.key] ?? ""}
          disabled={busy}
          onChange={(e) => setValues({ ...values, [def.key]: e.target.value })}
        />
      </label>
      {error?.field === def.key && (
        <p className="field-error" role="alert">
          {error.message}
        </p>
      )}
    </div>
  );

  return (
    <form
      className="described-form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
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
      <Button type="submit" disabled={busy || !ready(form, values)}>
        {form.submit.label}
      </Button>
    </form>
  );
}

function inputHas(form: FormDef, key: string | undefined) {
  return key !== undefined && inputs(form).some((i) => i.key === key);
}
