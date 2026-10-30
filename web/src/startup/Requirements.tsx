import { useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { ToolId } from "../api/generated/ToolId";
import type { ToolList } from "../api/generated/ToolList";
import type { ToolStatus } from "../api/generated/ToolStatus";
import { missingRequired, toolErrorText, toolName } from "../api/tools";
import { ExternalLink } from "../native/ExternalLink";
import { copyText, type Native } from "../native/native";
import { Button } from "../ui/Button";

interface Props {
  client: FleetClient;
  native: Native | null;
  /** The app may run an install itself: it is the app, and the daemon is on this machine. */
  canInstall: boolean;
  heading: string;
  list: ToolList;
  /** Why the list is not the daemon's latest answer, when it is not. */
  note: string | null;
  /** Each answer the daemon gives, so the app can take its PATH. */
  onAnswer: (list: ToolList) => void;
  onContinue: (list: ToolList) => void;
}

/** egui's Requirements step: what the daemon found of each tool, the way out for what it did not, Check again and Continue. */
export function Requirements({ client, native, canInstall, heading, list: first, note: firstNote, onAnswer, onContinue }: Props) {
  const [list, setList] = useState(first);
  const [note, setNote] = useState(firstNote);
  const [installing, setInstalling] = useState<ToolId | null>(null);
  const [rechecking, setRechecking] = useState(false);
  // A second Check again starts a fresh check; the answer to the first is no longer waited for.
  const asked = useRef(0);

  const apply = (l: ToolList) => {
    setList(l);
    setNote(null);
    onAnswer(l);
  };
  const fail = (e: unknown) => setNote(e instanceof Error ? e.message : String(e));

  const recheck = () => {
    const n = ++asked.current;
    const current = () => n === asked.current;
    setRechecking(true);
    void client
      .recheckTools()
      .then(
        (l) => current() && apply(l),
        (e: unknown) => current() && fail(e),
      )
      .finally(() => current() && setRechecking(false));
  };

  const install = (tool: ToolId, command: string) => {
    if (!native) return;
    setInstalling(tool);
    // When the window closes, whatever happened in it, the daemon says what is there now.
    void native
      .installTool(command)
      .then(() => client.recheckTools())
      .then(apply, fail)
      .finally(() => setInstalling(null));
  };

  // Nothing in flight: an Install click would otherwise race the answer.
  const installAllowed = canInstall && installing === null && !rechecking;
  return (
    <section className="startup-card wizard">
      <h2>{heading}</h2>
      <p>What the background service found on this machine.</p>
      {note && <p className="danger">{note}</p>}
      <div className="tool-rows">
        {list.tools.map((t) => (
          <ToolRow
            key={t.tool}
            status={t}
            native={native}
            installAllowed={installAllowed}
            installing={installing === t.tool}
            onInstall={(command) => install(t.tool, command)}
          />
        ))}
      </div>
      <div className="actions">
        <Button onClick={recheck}>Check again</Button>
        <Button variant="primary" onClick={() => onContinue(list)}>
          {missingRequired(list) ? "Continue without it" : "Continue"}
        </Button>
      </div>
    </section>
  );
}

function ToolRow(props: {
  status: ToolStatus;
  native: Native | null;
  installAllowed: boolean;
  installing: boolean;
  onInstall: (command: string) => void;
}) {
  const { status, native, installAllowed, installing, onInstall } = props;
  const name = toolName(status.tool);
  const e = status.error;
  if (e === null) return <p>{status.version === null ? `✓ ${name}` : `✓ ${name} ${status.version}`}</p>;
  const fix = e.kind === "not-logged-in" ? null : e.fix;
  return (
    <div className="tool-row">
      <p>{toolErrorText(e)}</p>
      {fix === null ? (
        <p className="note">Log in again from the persona's setup.</p>
      ) : (
        <>
          {fix.install !== null && installAllowed && (
            <div>
              <Button variant="primary" onClick={() => onInstall(fix.install!)}>
                Install
              </Button>
            </div>
          )}
          {fix.install !== null && (
            <div className="command">
              <code>{fix.install}</code>
              <Button onClick={() => void copyText(native, fix.install!)}>Copy</Button>
            </div>
          )}
          <ExternalLink href={fix.link} native={native} />
        </>
      )}
      {installing && <p className="note">Installing {name} in its own window; it is checked again when that window closes.</p>}
    </div>
  );
}
