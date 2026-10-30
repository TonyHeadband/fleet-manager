import { Button } from "../ui/Button";
import { Section } from "../ui/Section";

/** egui's `setup_wizard`: the heading, Relaunch setup wizard and what it does; on General and on a bare Extras. */
export function SetupWizard({ onRelaunch }: { onRelaunch: () => void }) {
  return (
    <Section title="Setup wizard">
      <Button onClick={onRelaunch}>Relaunch setup wizard</Button>
      <p className="note">Checks the tools Fleet Manager needs and installs what is missing; the folders can be changed there too.</p>
    </Section>
  );
}
