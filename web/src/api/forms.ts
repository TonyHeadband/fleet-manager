import type { FormDef } from "./generated/FormDef";
import type { InputDef } from "./generated/InputDef";

/** Each input's text by its key. */
export type FormValues = Record<string, string>;

const KINDS = new Set(["text", "date"]);
const ITEMS = new Set(["field", "group"]);
const RULES = new Set(["not_blank", "date", "not_before"]);

export function inputs(form: FormDef): InputDef[] {
  return form.items.flatMap((item) => (item.type === "field" ? [item] : item.type === "group" ? item.fields : []));
}

/** Mirrors `FormDef::is_known`: a form holding anything this build cannot draw is not drawn. */
export function isKnown(form: FormDef): boolean {
  return (
    form.items.every((i) => ITEMS.has(i.type)) &&
    inputs(form).every((i) => KINDS.has(i.kind) && (i.default == null || i.default.type === "today") && i.rules.every((r) => RULES.has(r.type)))
  );
}

export function initialValues(form: FormDef, today: string): FormValues {
  return Object.fromEntries(inputs(form).map((i) => [i.key, i.default?.type === "today" ? today : ""]));
}

/** Only not-blank is checked before sending; the daemon checks the rest and names the input. */
export function ready(form: FormDef, values: FormValues): boolean {
  return inputs(form).every((i) => !i.rules.some((r) => r.type === "not_blank") || (values[i.key] ?? "").trim() !== "");
}

export function formRequest(form: FormDef, params: Record<string, string>, values: FormValues) {
  const path = form.submit.path.replace(/\{(\w+)\}/g, (_, name: string) => encodeURIComponent(params[name] ?? ""));
  const body = Object.fromEntries(inputs(form).map((i) => [i.key, (values[i.key] ?? "").trim()]));
  return { method: form.submit.method, path, body };
}
