import type { FormDef } from "../api/generated/FormDef";
import { inputs } from "../api/forms";
import { FormField } from "../ui/Form";
import { SaveBar } from "../ui/SaveBar";
import { instructionsDirty, type AgentConfigApi } from "./agentConfig";
import { ErrorLine } from "../ui/ErrorLine";
import { ConfigPane } from "./ConfigPane";

// A port of crates/fleet-gui/src/views/management/instructions.rs: the text every agent's CLAUDE.md starts with. Its
// input is the daemon's `instructions` form; the pane keeps its own Save and Revert.

export function Instructions({ config, form }: { config: AgentConfigApi; form: FormDef | null }) {
  const st = config.st;
  // The pane shows without the schema, so "Loading…" and a load error do too; the field waits for it.
  const input = form && inputs(form).find((i) => i.key === "instructions");
  return (
    <ConfigPane answered={st.answered} loadError={st.loadError} width="editor">
      {input && <FormField def={input} value={st.instructions} onChange={config.setInstructions} />}
      {st.instructionsError !== null && <ErrorLine text={st.instructionsError} />}
      <SaveBar
        dirty={instructionsDirty(st)}
        busy={st.saving !== null}
        onSave={config.saveInstructions}
        onRevert={config.revertInstructions}
      />
    </ConfigPane>
  );
}
