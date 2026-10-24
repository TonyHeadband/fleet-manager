import { fieldValue, formatDuration } from "../api/schema";
import type { Agent } from "../api/generated/Agent";
import type { FieldDef } from "../api/generated/FieldDef";
import { Badge } from "./Badge";

function Value({ def, value, now }: { def: FieldDef; value: unknown; now: number }) {
  switch (def.kind) {
    case "badge":
      return (
        <Badge label={def.label} field={def.key}>
          {value === true ? def.label : String(value)}
        </Badge>
      );
    case "duration":
      return (
        <span className="duration" title={def.label} data-field={def.key}>
          {formatDuration(now - Number(value))}
        </span>
      );
    case "mono":
      return (
        <code className="mono" title={def.label} data-field={def.key}>
          {String(value)}
        </code>
      );
    case "text":
      return (
        <div className="text" data-field={def.key}>
          {String(value)}
        </div>
      );
  }
}

/** One schema-defined card field: the value is read from the agent and drawn by the field's kind. */
export function Field({ def, agent, now }: { def: FieldDef; agent: Agent; now: number }) {
  const value = fieldValue(agent, def.key);
  if (value === null || value === undefined || value === "" || value === false) return null;
  // A list field (the repos) shows one element per entry.
  const values = Array.isArray(value) ? value : [value];
  return (
    <>
      {values.map((v) => (
        <Value key={String(v)} def={def} value={v} now={now} />
      ))}
    </>
  );
}
