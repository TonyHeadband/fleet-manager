import { useEffect, useLayoutEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { FleetError, type FleetClient } from "../../api/client";
import { browserEventSource, subscribeRoomPosts, type EventSourceFactory } from "../../api/events";
import type { Agent } from "../../api/generated/Agent";
import type { AskReply } from "../../api/generated/AskReply";
import type { ChatBody } from "../../api/generated/ChatBody";
import type { Post } from "../../api/generated/Post";
import type { Room } from "../../api/generated/Room";
import { AskFrame } from "../../chat/AskFrame";
import { Entry } from "../../chat/ChatPane";
import { Feed } from "../../chat/Feed";
import { grouper, type Speaker } from "../../chat/speakers";
import { Composer } from "../../components/Composer";
import { ReplyBox } from "../../components/ReplyBox";
import { Avatar } from "../../ui/Avatar";
import { supervisorSeed } from "../../ui/avatarArt";
import { Button } from "../../ui/Button";
import { ChoiceMenu } from "../../ui/ChoiceMenu";
import { ErrorLine } from "../../ui/ErrorLine";

export type RoomsClient = Pick<FleetClient, "rooms" | "roomPosts" | "postToRoom" | "answerRoomAsk">;

interface Props {
  baseUrl: string;
  room: Room;
  client: RoomsClient;
  /** The drawer's persona: the owner's figure and the supervisor's traces are seeded from it. */
  persona: string;
  /** The fleet's agents, for each sender's traces (its role). */
  agents: Agent[];
  factory?: EventSourceFactory;
  /** Every room's post as it is made, so the list can count the other rooms' unread on the same stream. */
  onPost?: (p: Post) => void;
  /** The room was read: the daemon moved the owner's mark to its last post. */
  onRead?: () => void;
  /** System lines (Fleet Manager's own posts, as "landed: …", and announcements) are shown; off, only the talk is. */
  announcements?: boolean;
}

type OwnerKind = "message" | "announcement";
const KINDS: { id: OwnerKind; name: string }[] = [
  { id: "message", name: "Message" },
  { id: "announcement", name: "Announcement" },
];
const ALL = "all";
/** Within this many pixels of the end, the reader is at the latest: new posts keep them there. */
const AT_END = 2;
/** Live posts in the open room are marked read this long after the last of a burst, in one read. */
const READ_AFTER_MS = 400;

export const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const byId = (prev: Post[], next: Post[]) => {
  const map = new Map(prev.map((p) => [p.id, p]));
  for (const p of next) map.set(p.id, p);
  return [...map.values()].sort((a, b) => a.id - b.id);
};
const agentOf = (m: Post["sender"] | null | undefined) => (m && typeof m === "object" ? m.agent : null);

/**
 * A room as a group chat, drawn with the agent chat's items: the owner's posts on the right, agents' messages as
 * replies under their name and traces, announcements and Fleet Manager's posts as system lines, and asks with their
 * answer (an open ask in the accent outline, an answered one in the text colour). Below, the owner's post box: who to (everyone or one member) and what kind.
 */
export function RoomChat(props: Props) {
  // Another room starts afresh: its own posts, draft and pickers.
  return <Chat key={props.room.id} {...props} />;
}

function Chat({ baseUrl, room, client, persona, agents, factory = browserEventSource, onPost, onRead, announcements = true }: Props) {
  const [posts, setPosts] = useState<Post[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // What the owner's own answers said, until the answer's post arrives.
  const [mine, setMine] = useState<ReadonlyMap<number, string>>(new Map());
  const [to, setTo] = useState<string>(ALL);
  const [kind, setKind] = useState<OwnerKind>("message");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [postError, setPostError] = useState<string | null>(null);
  const feed = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  // The stream outlives renders; the latest callbacks are read through these.
  const heard = useRef({ onPost, onRead });
  heard.current = { onPost, onRead };

  // A read moves the owner's mark on the server; at most one is in flight, and one asked meanwhile follows it.
  const reading = useRef(false);
  const readAgain = useRef(false);
  const pendingRead = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const read = () => {
    if (reading.current) {
      readAgain.current = true;
      return;
    }
    reading.current = true;
    client
      .roomPosts(room.id)
      .then(
        (ps) => {
          setPosts((prev) => byId(prev, ps));
          setLoaded(true);
          heard.current.onRead?.();
        },
        (e) => setError(message(e)),
      )
      .finally(() => {
        reading.current = false;
        if (readAgain.current) {
          readAgain.current = false;
          read();
        }
      });
  };
  const readSoon = () => {
    clearTimeout(pendingRead.current);
    pendingRead.current = setTimeout(() => {
      pendingRead.current = undefined;
      read();
    }, READ_AFTER_MS);
  };
  useEffect(() => {
    read();
    // Leaving the room with a read still waiting makes it now, so what was shown is not left unread.
    return () => {
      if (pendingRead.current === undefined) return;
      clearTimeout(pendingRead.current);
      read();
    };
    // Once per room: the component is keyed by it.
  }, []);

  useEffect(
    () =>
      subscribeRoomPosts(
        baseUrl,
        (p) => {
          if (p.room === room.id) {
            setPosts((prev) => byId(prev, [p]));
            readSoon();
          }
          heard.current.onPost?.(p);
        },
        factory,
      ),
    [baseUrl, room.id, factory],
  );

  useLayoutEffect(() => {
    const el = feed.current;
    if (el && atEnd.current) el.scrollTop = el.scrollHeight;
  }, [posts, mine]);

  // An orchestrator posts as its task: its agent is the task's own id, or names the task.
  const agentFor = (id: string | null) => (id === null ? undefined : (agents.find((a) => a.id === id) ?? agents.find((a) => a.kind === "orchestrator" && a.task === id)));
  const speakerOf = (p: Post): Speaker => {
    if (p.sender === "owner") return { id: "owner", name: p.sender_name, avatar: <Avatar seed={persona} title={persona} /> };
    const sender = agentFor(agentOf(p.sender));
    const id = agentOf(p.sender) ?? p.sender_name;
    if (sender?.kind === "supervisor") return { id, name: p.sender_name, avatar: <Avatar kind="supervisor" seed={supervisorSeed(persona)} title="supervisor" /> };
    const role = sender?.role;
    return { id, name: p.sender_name, avatar: role ? <Avatar kind="specialist" seed={role} title={role} /> : <Avatar /> };
  };
  // Posts are in order; a post that is no message (a system line, an ask) ends the run before it.
  const talk = grouper();
  const item = (p: Post, body: ChatBody) => {
    const speaker = body.type === "reply" || p.sender === "owner" ? speakerOf(p) : undefined;
    return <Entry key={p.id} item={{ seq: p.id, at: p.at, body }} speaker={speaker} head={talk.next(speaker, p.at)} />;
  };

  const answers = new Map(posts.flatMap((p) => (p.kind === "answer" && p.reply_to != null ? [[p.reply_to, p] as const] : [])));
  const asked = new Set(posts.filter((p) => p.kind === "ask").map((p) => p.id));
  const rows: ReactNode[] = posts.flatMap((p) => {
    // An answer is drawn under its ask; one whose ask is not loaded stands on its own.
    if (p.kind === "answer" && p.reply_to != null && asked.has(p.reply_to)) return [];
    if (!announcements && (p.sender === "daemon" || p.kind === "announcement")) return [];
    if (p.sender === "daemon") return [item(p, { type: "prompt", text: p.text, from: "system" })];
    if (p.kind === "announcement") return [item(p, { type: "prompt", text: `${p.sender_name} · ${p.text}`, from: "system" })];
    if (p.sender === "owner") return [item(p, { type: "prompt", text: p.text, from: "owner" })];
    if (p.kind === "ask") {
      talk.reset();
      const forOwner = p.to === "owner" || agentFor(agentOf(p.to))?.kind === "supervisor";
      return [
        <RoomAsk
          key={p.id}
          ask={p}
          speaker={speakerOf(p)}
          answer={answers.get(p.id)?.text ?? mine.get(p.id) ?? null}
          answerable={forOwner}
          client={client}
          onAnswered={(said) => setMine((m) => new Map(m).set(p.id, said))}
          onStale={read}
        />,
      ];
    }
    return [item(p, { type: "reply", text: p.text })];
  });

  // The room's agents, as the daemon lists them: never the owner or the daemon.
  const recipients = [{ id: ALL, name: "All" }, ...room.members];

  const send = (body: string) => {
    setBusy(true);
    setPostError(null);
    client.postToRoom(room.id, { kind, to: to === ALL ? null : { agent: to }, text: body }).then(
      (p) => {
        atEnd.current = true;
        setPosts((prev) => byId(prev, [p]));
        setText("");
        setBusy(false);
      },
      (e) => {
        setPostError(message(e));
        setBusy(false);
      },
    );
  };

  return (
    <>
      <Feed
        feedRef={feed}
        label="Room posts"
        onScroll={(el) => {
          atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight <= AT_END;
        }}
      >
          {error && <ErrorLine text={error} />}
          {loaded && rows.length === 0 && <p className="chat-empty">Nothing yet: the room starts with its first post.</p>}
          {rows}
      </Feed>
      <Composer label="Post" initiallyOpen={false} draft={text.trim() !== ""}>
        <div className="room-post-pickers">
          <ChoiceMenu label="To" name="Recipient" width="menu" items={recipients} value={to} onPick={setTo} />
          <ChoiceMenu label="Kind" name="Post kind" width="menu" items={KINDS} value={kind} onPick={setKind} />
        </div>
        {postError && <ErrorLine text={postError} />}
        <ReplyBox label="Post" text={text} onText={setText} onSend={send} busy={busy} />
      </Composer>
    </>
  );
}

interface AskProps {
  ask: Post;
  speaker: Speaker;
  /** The answer's text, once it has one. */
  answer: string | null;
  /** Asked of the owner or the supervisor: the owner may answer it here. */
  answerable: boolean;
  client: RoomsClient;
  onAnswered: (said: string) => void;
  /** The daemon refused the answer as already answered or stale: read the room again. */
  onStale: () => void;
}

/**
 * An ask as the agent chat draws one (only an open ask is in the accent outline): the question under its sender, then
 * its answer. Open and asked of the owner or the supervisor, it takes an answer; a breakdown's ask is approved or sent back with changes.
 */
function RoomAsk({ ask, speaker, answer, answerable, client, onAnswered, onStale }: AskProps) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const breakdown = ask.breakdown_version ?? null;
  const ready = !sending && text.trim() !== "";

  const reply = async (body: AskReply, said: string) => {
    setSending(true);
    setError(null);
    try {
      await client.answerRoomAsk(ask.id, body);
      onAnswered(said);
    } catch (e) {
      setError(message(e));
      if (e instanceof FleetError && e.status === 409) onStale();
    } finally {
      setSending(false);
    }
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ready) reply({ text: text.trim() }, text.trim());
  };

  const open = answer === null && ask.answered_at == null;
  return (
    <AskFrame caption={breakdown === null ? "Asks" : `Asks · breakdown v${breakdown}`} summary={ask.text} answered={answer ?? (open ? null : "")} error={error}>
      <Entry item={{ seq: ask.id, at: ask.at, body: { type: "reply", text: ask.text } }} speaker={speaker} />
      {open && answerable && (
        <form className="chat-ask-free" onSubmit={submit}>
          <input
            type="text"
            aria-label={breakdown === null ? "Answer" : "Changes"}
            placeholder={breakdown === null ? "Answer…" : "Changes…"}
            value={text}
            onChange={(e) => setText(e.target.value)}
            disabled={sending}
          />
          {breakdown === null ? (
            <Button type="submit" variant="primary" disabled={!ready}>
              Send
            </Button>
          ) : (
            <>
              <Button variant="primary" disabled={sending} onClick={() => reply({ approve: true }, "Approved")}>
                Approve
              </Button>
              <Button type="submit" disabled={!ready}>
                Request changes
              </Button>
            </>
          )}
        </form>
      )}
    </AskFrame>
  );
}
