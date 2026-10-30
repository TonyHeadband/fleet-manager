// fleet-client's login scripts (crates/fleet-client/src/login.rs), for a browser, which has no terminal to open: it
// shows the script to run on the daemon's machine. The app builds them in Rust. `login.test.ts` holds the two to
// the same output, and the variables a login unsets come from fleet-api through the generated file.
import { ambient } from "../generated/login.json";
import type { Persona } from "./generated/Persona";
import type { PersonaList } from "./generated/PersonaList";

type LoginDirs = Pick<Persona, "claude_dir" | "gh_dir" | "host">;

/** Quotes for a POSIX shell; a leading `~` or `~/` stays outside the quotes so the shell still expands it. */
function shellQuote(s: string): string {
  const [head, rest] = s === "~" ? ["~", ""] : s.startsWith("~/") ? ["~/", s.slice(2)] : ["", s];
  if (head === "" && rest === "") return "''";
  const plain = rest !== "" && /^[A-Za-z0-9_./\-+@%:,=]+$/.test(rest);
  return head + (plain || rest === "" ? rest : `'${rest.replaceAll("'", "'\\''")}'`);
}

const envPrefix = () => `env ${ambient.map((v) => `-u ${shellQuote(v)} `).join("")}`;

export function claudeLoginScript(p: LoginDirs, claude: string[], tasksRoot: string): string {
  return `cd ${shellQuote(tasksRoot)} && ${envPrefix()}CLAUDE_CONFIG_DIR=${shellQuote(p.claude_dir)} ${claude.map(shellQuote).join(" ")}`;
}

export function ghLoginScript(p: LoginDirs, gh: string): string {
  const gitconfig = `${p.gh_dir.replace(/\/+$/, "")}/gitconfig`;
  return (
    `${envPrefix()}GH_CONFIG_DIR=${shellQuote(p.gh_dir)} GIT_CONFIG_GLOBAL=${shellQuote(gitconfig)} ${shellQuote(gh)} ` +
    `auth login --hostname ${shellQuote(p.host)} --git-protocol https --insecure-storage`
  );
}

/** PersonaList's `claude_login_command` and `gh_login_program`: where the daemon found the CLIs, else as configured. */
export const claudeCommand = (l: PersonaList) =>
  l.claude_path === null || l.claude_command.length === 0 ? l.claude_command : [l.claude_path, ...l.claude_command.slice(1)];
export const ghProgram = (l: PersonaList) => l.gh_path ?? l.gh_command;
