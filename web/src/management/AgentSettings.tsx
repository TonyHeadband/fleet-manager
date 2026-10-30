import { PairRowsEditor, RowsEditor } from "../ui/RowsEditor";
import { SaveBar } from "../ui/SaveBar";
import { Section } from "../ui/Section";
import { settingsDirty, type AgentConfigApi } from "./agentConfig";
import { ErrorLine } from "../ui/ErrorLine";
import { ConfigPane } from "./ConfigPane";

// A port of crates/fleet-gui/src/views/management/agent_settings.rs: the permissions and environment every agent gets,
// whatever its persona.

export function AgentSettings({ config }: { config: AgentConfigApi }) {
  const st = config.st;
  const d = st.settings;
  const set = config.setSettings;
  return (
    <ConfigPane answered={st.answered} loadError={st.loadError} width="settings">
      {st.saved.settings_error && <ErrorLine text={`${st.saved.settings_error}. Saving valid settings replaces the file.`} />}
      <p className="note">Written into each agent's Claude settings, on top of what its persona and task add.</p>
      <Section title="Allow rules">
        <RowsEditor rows={d.allow} noun="allow rule" hint="Bash(git status:*)" onChange={(allow) => set({ ...d, allow })} />
      </Section>
      <Section title="Deny rules">
        <RowsEditor rows={d.deny} noun="deny rule" hint="Read(./.env)" onChange={(deny) => set({ ...d, deny })} />
      </Section>
      <Section title="Environment">
        <p className="note">A repo's `env:` in `.fleet/workflow.yml` wins over these; Products shows what each repo's tasks run with.</p>
        <PairRowsEditor rows={d.env} onChange={(env) => set({ ...d, env })} />
      </Section>
      {st.settingsError !== null && <ErrorLine text={st.settingsError} />}
      <SaveBar
        dirty={settingsDirty(st)}
        force={!!st.saved.settings_error}
        busy={st.saving !== null}
        onSave={config.saveSettings}
        onRevert={config.revertSettings}
      />
    </ConfigPane>
  );
}
