import type { ReactNode } from "react";
import type { Agent } from "../api/generated/Agent";
import type { ChatItem } from "../api/generated/ChatItem";
import type { Persona } from "../api/generated/Persona";
import { nickname } from "../board/groups";
import { Avatar } from "../ui/Avatar";

/** Who said a message: `id` tells one speaker from another, for collapsing a run under one header. */
export interface Speaker {
  id: string;
  name: string;
  avatar?: ReactNode;
}

/** The speakers of a one-to-one chat: the owner's prompts and answers, and the agent's replies. */
export interface ChatSpeakers {
  owner: Speaker;
  agent: Speaker;
}

/** A run of messages from one speaker collapses under one header while each follows the last within this many seconds. */
const RUN_SECS = 300;

/** The owner, as the active persona: its figure and its name. */
export function ownerSpeaker(persona: string | null, personas: Persona[]): Speaker {
  const name = personas.find((p) => p.id === persona)?.name ?? persona ?? "Owner";
  return { id: "owner", name, avatar: <Avatar title={name} seed={persona} /> };
}

/** An agent, by its specialist or orchestrator type: "NICKNAME · name" where its persona gave it a nickname. */
export function agentSpeaker(a: Agent, personas: Persona[]): Speaker {
  const role = a.role ?? null;
  const name = role ?? "agent";
  const nick = nickname(a, personas);
  return {
    id: a.id,
    name: nick ? `${nick} · ${name}` : name,
    avatar: role ? <Avatar kind="specialist" seed={role} title={role} /> : <Avatar />,
  };
}

/** The supervisor of a persona: one each, drawn as a specialist seeded by the persona. */
export function supervisorSpeaker(persona: string | null): Speaker {
  return { id: "supervisor", name: "Supervisor", avatar: <Avatar kind="specialist" seed={persona && `supervisor:${persona}`} title="supervisor" /> };
}

/** Who said a one-to-one chat's item; calls, markers, events and system lines have no speaker. */
export function speakerOf(item: ChatItem, speakers: ChatSpeakers): Speaker | undefined {
  const body = item.body;
  if (body.type === "reply") return speakers.agent;
  return body.type === "prompt" && body.from === "owner" ? speakers.owner : undefined;
}

/**
 * Decides which messages open with a header: one from a new speaker, more than five minutes after the last message,
 * or after anything that is not a message (`next` with no speaker).
 */
export function grouper() {
  let last: { id: string; at: number } | null = null;
  return {
    next(speaker: Speaker | undefined, at: number): boolean {
      if (!speaker) {
        last = null;
        return false;
      }
      const head = !last || last.id !== speaker.id || at - last.at > RUN_SECS;
      last = { id: speaker.id, at };
      return head;
    },
    reset() {
      last = null;
    },
  };
}
