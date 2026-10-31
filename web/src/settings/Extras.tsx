import type { Native } from "../native/native";
import { ToolRows } from "../startup/Requirements";
import type { ToolCheck } from "../startup/toolCheck";
import { Button } from "../ui/Button";
import { Section } from "../ui/Section";
import { SetupWizard } from "./SetupWizard";

// A port of crates/fleet-gui/src/views/settings/extras.rs: the optional tools with Install; with none, the required
// tools read-only and the way to the wizard that installs them.

export function Extras({ check, native, onRelaunch }: { check: ToolCheck; native: Native | null; onRelaunch: () => void }) {
  const { list, note } = check;
  const optional = list.tools.filter((t) => !t.required);
  return (
    <div className="tabbed-page settings-page">
      {note && <p className="danger">{note}</p>}
      {list.tools.length === 0 && check.checking ? (
        <p className="note">Checking the tools…</p>
      ) : (
        <>
          {optional.length > 0 ? (
            <Section title="Extras">
              <ToolRows tools={optional} check={check} native={native} install={check.canInstall} />
            </Section>
          ) : (
            <>
              <Section title="Required tools">
                {/* Installing them is the wizard's. */}
                <ToolRows tools={list.tools.filter((t) => t.required)} check={check} native={native} install={false} />
              </Section>
              <SetupWizard onRelaunch={onRelaunch} />
            </>
          )}
          <div className="extras-check">
            <Button onClick={check.recheck}>Check again</Button>
          </div>
        </>
      )}
    </div>
  );
}
