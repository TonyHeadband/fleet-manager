import type { ToolId } from "../api/generated/ToolId";
import { toolName } from "../api/tools";

// crates/fleet-gui/src/shell.rs: the rows the intro counts before the answer, and how long it names each row of it.
export const TOOL_ROWS: ToolId[] = ["git", "git-bash", "claude", "gh"];
export const ROW_MS = 150;
/** How often the browser asks again while the daemon does not answer. */
export const RETRY_MS = 1000;

const NAMES: Record<ToolId, string> = {
  git: "Git",
  "git-bash": "Git Bash",
  claude: "Claude Code",
  gh: "GitHub CLI",
};

export const checkingLabel = (tool: ToolId) => `Checking ${NAMES[tool]}…`;

export const STARTING = "Starting the background service…";

/** What the intro shows: the step under way and how far along the bar is. */
export interface IntroProgress {
  label: string;
  done: number;
  total: number;
}

/** Which of the wizard's optional steps this run shows, to count them: egui's `WizardSteps`. */
export interface WizardSteps {
  folders: boolean;
  requirements: boolean;
}

export type WizardStep = "folders" | "requirements" | "identity" | "login";

/** "Step N of M · `title`" for `step`. */
export function wizardHeading(steps: WizardSteps, step: WizardStep, title: string): string {
  const before = Number(steps.folders) + Number(steps.requirements);
  const n = { folders: 1, requirements: 1 + Number(steps.folders), identity: before + 1, login: before + 2 }[step];
  return `Step ${n} of ${before + 2} · ${title}`;
}
