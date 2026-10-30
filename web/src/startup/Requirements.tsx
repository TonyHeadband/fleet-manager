import type { ReactNode } from "react";
import type { ToolError } from "../api/generated/ToolError";
import type { ToolId } from "../api/generated/ToolId";
import type { ToolList } from "../api/generated/ToolList";
import type { ToolStatus } from "../api/generated/ToolStatus";
import { missingRequired, toolErrorText, toolName } from "../api/tools";
import { ExternalLink } from "../native/ExternalLink";
import { copyText, type Native } from "../native/native";
import { Button } from "../ui/Button";
import type { ToolCheck } from "./toolCheck";

interface Props {
  native: Native | null;
  check: ToolCheck;
  heading: string;
  onContinue: (list: ToolList) => void;
}

/** egui's Requirements step: what the daemon found of each tool, the way out for what it did not, Check again and Continue. */
export function Requirements({ native, check, heading, onContinue }: Props) {
  return (
    <section className="startup-card wizard">
      <h2>{heading}</h2>
      <RequirementsBody native={native} check={check} onContinue={onContinue} />
    </section>
  );
}

/** The step under its heading: the lead line, the note, the rows, Check again and Continue (egui `requirements_ui`). */
export function RequirementsBody({ native, check, onContinue }: Omit<Props, "heading">) {
  return (
    <>
      <p>What the background service found on this machine.</p>
      {check.checking && check.list.tools.length === 0 && <p className="note">Checking the tools…</p>}
      {check.note && <p className="danger">{check.note}</p>}
      <ToolRows tools={check.list.tools} check={check} native={native} install={check.canInstall} />
      <div className="actions">
        <Button onClick={check.recheck}>Check again</Button>
        <Button variant="primary" onClick={() => onContinue(check.list)}>
          {missingRequired(check.list) ? "Continue without it" : "Continue"}
        </Button>
      </div>
    </>
  );
}

/** egui's `tool_rows`: each tool's state, with Install when `install` allows it. */
export function ToolRows({ tools, check, native, install }: { tools: ToolStatus[]; check: ToolCheck; native: Native | null; install: boolean }) {
  return (
    <div className="tool-rows">
      {tools.map((t) => (
        <ToolRow
          key={t.tool}
          status={t}
          native={native}
          installAllowed={install}
          installing={check.installing === t.tool}
          onInstall={() => check.install(t.tool)}
        />
      ))}
    </div>
  );
}

function ToolRow(props: {
  status: ToolStatus;
  native: Native | null;
  installAllowed: boolean;
  installing: boolean;
  onInstall: () => void;
}) {
  const { status, native, installAllowed, installing, onInstall } = props;
  const name = toolName(status.tool);
  if (status.error === null) return <p>{status.version === null ? `✓ ${name}` : `✓ ${name} ${status.version}`}</p>;
  const install = installAllowed && status.error.kind !== "not-logged-in" && status.error.fix.install !== null;
  return (
    <div className="tool-row">
      <ToolProblem error={status.error} native={native}>
        {install && (
          <div>
            <Button variant="primary" onClick={onInstall}>
              Install
            </Button>
          </div>
        )}
      </ToolProblem>
      {installing && <p className="note">Installing {name} in its own window; it is checked again when that window closes.</p>}
    </div>
  );
}

/**
 * egui's `tool_error_ui`: the problem, then the way out (`children`, then the command to copy and the link). A login
 * that is only logged out has no fix.
 */
export function ToolProblem({ error, native, children }: { error: ToolError; native: Native | null; children?: ReactNode }) {
  const fix = error.kind === "not-logged-in" ? null : error.fix;
  return (
    <>
      <p>{toolErrorText(error)}</p>
      {fix === null ? (
        <p className="note">Log in again from the persona's setup.</p>
      ) : (
        <>
          {children}
          {fix.install !== null && <CopyLine text={fix.install} native={native} />}
          <ExternalLink href={fix.link} native={native} />
        </>
      )}
    </>
  );
}

/** A command on one line, with Copy. */
export function CopyLine({ text, native }: { text: string; native: Native | null }) {
  return (
    <div className="command">
      <code>{text}</code>
      <Button onClick={() => void copyText(native, text)}>Copy</Button>
    </div>
  );
}
