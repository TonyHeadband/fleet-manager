import type { ToolError } from "./generated/ToolError";
import type { ToolId } from "./generated/ToolId";
import type { ToolList } from "./generated/ToolList";

const TOOL_NAMES: Record<ToolId, string> = { git: "Git", "git-bash": "Git Bash", claude: "Claude Code", gh: "GitHub CLI (gh)" };

/** Lite's `ToolId::name`. */
export const toolName = (tool: ToolId) => TOOL_NAMES[tool];

/** Lite's `ToolError` display. */
export function toolErrorText(e: ToolError): string {
  const name = TOOL_NAMES[e.tool];
  switch (e.kind) {
    case "not-installed":
      return `${name} is not installed`;
    case "too-old":
      return `${name} ${e.found} is too old: ${e.needed} or newer is needed`;
    case "not-logged-in":
      return e.persona === null ? `${name} is not logged in` : `${name} is not logged in for ${e.persona}`;
    case "broken":
      return `${name} is installed but does not work: ${e.detail}`;
  }
}

/** Lite's `ToolList::missing_required`. */
export const missingRequired = (list: ToolList) => list.tools.some((t) => t.required && t.error !== null);
