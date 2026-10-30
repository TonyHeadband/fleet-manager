import type { ToolId } from "../api/generated/ToolId";

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
