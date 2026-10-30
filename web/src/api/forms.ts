import type { FormDef } from "./generated/FormDef";
import type { InputDef } from "./generated/InputDef";
import type { SubmitDef } from "./generated/SubmitDef";
import type { UiSchema } from "./generated/UiSchema";

/** Each input's text by its key. */
export type FormValues = Record<string, string>;

/** The tags this build draws; checked against the daemon's `form-vocabulary.json` so a new one cannot drift. */
export const KNOWN = {
  items: new Set(["field", "group"]),
  kinds: new Set(["text", "date", "multiline", "markdown", "choice"]),
  sources: new Set(["product_repos"]),
  defaults: new Set(["today"]),
  rules: new Set(["not_blank", "date", "not_before"]),
};

export function inputs(form: FormDef): InputDef[] {
  return form.items.flatMap((item) => (item.type === "field" ? [item] : item.type === "group" ? item.fields : []));
}

/** Mirrors `FormDef::is_known`: a form holding anything this build cannot draw is not drawn. */
export function isKnown(form: FormDef): boolean {
  return (
    form.items.every((i) => KNOWN.items.has(i.type)) &&
    inputs(form).every(
      (i) =>
        KNOWN.kinds.has(i.kind) &&
        (i.default == null || KNOWN.defaults.has(i.default.type)) &&
        i.rules.every((r) => KNOWN.rules.has(r.type)) &&
        (i.kind !== "choice" || (i.source != null && KNOWN.sources.has(i.source.type))),
    )
  );
}

/** The daemon's form when this build can draw it; else the caller uses its own copy (Lite's `usable_form`). */
export function usableForm(schema: UiSchema, id: string): FormDef | undefined {
  const form = schema.forms.find((f) => f.id === id);
  return form && isKnown(form) ? form : undefined;
}

export function initialValues(form: FormDef, today: string): FormValues {
  return Object.fromEntries(inputs(form).map((i) => [i.key, i.default?.type === "today" ? today : ""]));
}

/** Only not-blank is checked before sending; the daemon checks the rest and names the input. */
export function ready(form: FormDef, values: FormValues): boolean {
  return inputs(form).every((i) => !i.rules.some((r) => r.type === "not_blank") || (values[i.key] ?? "").trim() !== "");
}

/** `submit`, then the further actions. */
export function submits(form: FormDef): SubmitDef[] {
  return [form.submit, ...(form.more ?? [])];
}

/** `ready`, plus the inputs this action alone needs. */
export function readyFor(form: FormDef, submit: SubmitDef, values: FormValues): boolean {
  return ready(form, values) && (submit.needs ?? []).every((k) => (values[k] ?? "").trim() !== "");
}

/** Multiline and Markdown text is sent as typed: leading indentation can be a code block. */
const sentAsTyped = (i: InputDef) => i.kind === "multiline" || i.kind === "markdown";

/** The action's request: inputs (one-line ones trimmed), its fixed values, and each context key `context` has. */
export function formRequest(
  form: FormDef,
  submit: SubmitDef,
  params: Record<string, string>,
  values: FormValues,
  context: Record<string, unknown> = {},
) {
  const path = submit.path.replace(/\{(\w+)\}/g, (_, name: string) => encodeURIComponent(params[name] ?? ""));
  const body: Record<string, unknown> = Object.fromEntries(
    inputs(form).map((i) => {
      const value = values[i.key] ?? "";
      return [i.key, sentAsTyped(i) ? value : value.trim()];
    }),
  );
  Object.assign(body, submit.fixed ?? {});
  for (const key of submit.context ?? []) {
    if (context[key] !== undefined) body[key] = context[key];
  }
  return { method: submit.method, path, body };
}
