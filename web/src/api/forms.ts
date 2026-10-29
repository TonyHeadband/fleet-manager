import type { FormDef } from "./generated/FormDef";
import type { InputDef } from "./generated/InputDef";
import type { SubmitDef } from "./generated/SubmitDef";

/** Each input's text by its key. */
export type FormValues = Record<string, string>;

const KINDS = new Set(["text", "date", "multiline", "markdown", "choice"]);
const SOURCES = new Set(["product_repos"]);
const ITEMS = new Set(["field", "group"]);
const RULES = new Set(["not_blank", "date", "not_before"]);

export function inputs(form: FormDef): InputDef[] {
  return form.items.flatMap((item) => (item.type === "field" ? [item] : item.type === "group" ? item.fields : []));
}

/** Mirrors `FormDef::is_known`: a form holding anything this build cannot draw is not drawn. */
export function isKnown(form: FormDef): boolean {
  return (
    form.items.every((i) => ITEMS.has(i.type)) &&
    inputs(form).every(
      (i) =>
        KINDS.has(i.kind) &&
        (i.default == null || i.default.type === "today") &&
        i.rules.every((r) => RULES.has(r.type)) &&
        (i.kind !== "choice" || (i.source != null && SOURCES.has(i.source.type))),
    )
  );
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

/** The action's request: trimmed inputs, its fixed values, and each context key `context` has. */
export function formRequest(
  form: FormDef,
  submit: SubmitDef,
  params: Record<string, string>,
  values: FormValues,
  context: Record<string, unknown> = {},
) {
  const path = submit.path.replace(/\{(\w+)\}/g, (_, name: string) => encodeURIComponent(params[name] ?? ""));
  const body: Record<string, unknown> = Object.fromEntries(inputs(form).map((i) => [i.key, (values[i.key] ?? "").trim()]));
  Object.assign(body, submit.fixed ?? {});
  for (const key of submit.context ?? []) {
    if (context[key] !== undefined) body[key] = context[key];
  }
  return { method: submit.method, path, body };
}
