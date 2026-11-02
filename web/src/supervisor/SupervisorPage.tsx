import { useCallback, useEffect, useState } from "react";
import type { FleetClient } from "../api/client";
import type { EventSourceFactory } from "../api/events";
import type { Agent } from "../api/generated/Agent";
import type { Conversation } from "../api/generated/Conversation";
import type { Persona } from "../api/generated/Persona";
import type { Task } from "../api/generated/Task";
import { ChatPane, type ChatClient } from "../chat/ChatPane";
import { ownerSpeaker, supervisorSpeaker } from "../chat/speakers";
import { ReplyBox } from "../components/ReplyBox";
import { navigate, supervisorHref, useSupervisorConversation, useSupervisorRoom } from "../shell/routes";
import { ErrorLine } from "../ui/ErrorLine";
import { ConversationSelector } from "./ConversationSelector";
import { AGENTS_BY_STATUS } from "./widgets/AgentsByStatus";
import type { RoomsClient } from "./widgets/RoomChat";
import { ROOMS } from "./widgets/Rooms";
import { STORAGE } from "./widgets/Storage";
import "./supervisor.css";

export type SupervisorClient = ChatClient & RoomsClient & Pick<FleetClient, "conversations" | "newConversation" | "sendToConversation">;

interface Props {
  baseUrl: string;
  client: SupervisorClient;
  /** The drawer's persona: one supervisor each. */
  persona: string | null;
  /** The personas, for the owner's name in the chat. */
  personas: Persona[];
  /** The fleet's agents, for the conversations' live status. */
  agents: Agent[];
  /** The task list the app already keeps; empty until it loads. */
  tasks: Task[];
  /** The fleet snapshot has arrived. */
  loaded: boolean;
  factory?: EventSourceFactory;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Layout A: the chat at a fixed width with the conversation picker above it, the widgets' grid filling the rest. */
export function SupervisorPage(props: Props) {
  // Another persona is another supervisor: its own conversations and draft.
  return <Page key={props.persona ?? ""} {...props} />;
}

function Page({ baseUrl, client, persona, personas, agents, tasks, loaded, factory }: Props) {
  const [list, setList] = useState<Conversation[] | null>(null);
  // "+ New" was pressed: the chat is empty until the first message makes the conversation.
  const [blank, setBlank] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const named = useSupervisorConversation();
  // The room the address opens in the Rooms widget survives the page naming its conversation.
  const room = useSupervisorRoom();

  // Task marks and `live` follow the fleet: ask again when a supervisor's or a task's status moves.
  const standing = agents.map((a) => `${a.id}:${a.status}`).join(" ");
  const reload = useCallback(() => {
    if (persona === null) return;
    client.conversations(persona).then(setList, (e) => setError(message(e)));
  }, [client, persona]);
  useEffect(reload, [reload, standing]);

  const conversations = list ?? [];
  const known = named !== null && conversations.some((c) => c.id === named) ? named : null;
  const current = blank ? null : (known ?? conversations[0]?.id ?? null);
  // An address left by another persona (or a deleted conversation) is let go once the list is known.
  useEffect(() => {
    if (list !== null && named !== null && known === null) navigate(supervisorHref(null, room), true);
  }, [list, named, known, room]);

  // The latest conversation, shown for an address without one, is named in it so the rest of the app can tell what is
  // on screen.
  useEffect(() => {
    if (!blank && named === null && current !== null) navigate(supervisorHref(current, room), true);
  }, [blank, named, current, room]);

  const pick = (id: string) => {
    setBlank(false);
    navigate(supervisorHref(id));
  };
  const startNew = () => {
    setBlank(true);
    navigate(supervisorHref());
  };

  const send = (body: string) => {
    if (persona === null) return;
    setBusy(true);
    setError(null);
    const sent = current
      ? client.sendToConversation(current, body).then(reload)
      : client.newConversation(persona, body).then((made) => {
          setList((l) => [made, ...(l ?? []).filter((c) => c.id !== made.id)]);
          setBlank(false);
          navigate(supervisorHref(made.id));
        });
    sent.then(
      () => {
        setText("");
        setBusy(false);
      },
      (e) => {
        setError(message(e));
        setBusy(false);
      },
    );
  };

  return (
    <div className="supervisor">
      <section className="supervisor-grid outlined" aria-label="Fleet widgets">
        {persona !== null &&
          [AGENTS_BY_STATUS, STORAGE, ROOMS].map((w) => (
            <div key={w.id} className="supervisor-cell" data-cell={w.id}>
              {w.render({ persona, agents, tasks, loaded, baseUrl, client, factory })}
            </div>
          ))}
      </section>
      <section className="supervisor-chat outlined" aria-label="Supervisor chat">
        <header className="supervisor-chat-head">
          <ConversationSelector conversations={conversations} current={current} onPick={pick} onNew={startNew} />
        </header>
        <div className="supervisor-chat-body">
          {current !== null ? (
            <ChatPane baseUrl={baseUrl} agentId={current} client={client} factory={factory} speakers={{ owner: ownerSpeaker(persona, personas), agent: supervisorSpeaker(persona) }} />
          ) : (
            list !== null && <p className="supervisor-empty">Start a conversation with your supervisor.</p>
          )}
        </div>
        {error && <ErrorLine text={error} />}
        {persona !== null && <ReplyBox label="Message" text={text} onText={setText} onSend={send} busy={busy} />}
      </section>
    </div>
  );
}
