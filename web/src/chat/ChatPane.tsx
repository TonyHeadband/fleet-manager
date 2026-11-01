import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { FleetClient } from "../api/client";
import { browserEventSource, subscribeChat, type EventSourceFactory } from "../api/events";
import type { ChatAsk } from "../api/generated/ChatAsk";
import type { ChatBody } from "../api/generated/ChatBody";
import type { ChatItem } from "../api/generated/ChatItem";
import { clock } from "../shell/Footer";
import { ErrorLine } from "../ui/ErrorLine";
import { PlusMinus } from "../ui/PlusMinus";
import { AskMessage } from "./AskMessage";
import { Markdown } from "./Markdown";
import { blocks, mergeItems, opensItself, type Section } from "./sections";
import "./chat.css";

export type ChatClient = Pick<FleetClient, "chatHistory" | "answerAsk">;

interface Props {
  baseUrl: string;
  agentId: string;
  client: ChatClient;
  factory?: EventSourceFactory;
}

type Call = Extract<ChatBody, { type: "call" }>;

/** The daemon's page: each scroll to the top asks for this many items more. */
const PAGE = 200;
/** Within this many pixels of the end, the reader is at the latest: new messages keep them there. */
const AT_END = 2;

/** An ask in the chat. An answered one stays where it was answered (`after`, the last item's seq then). */
interface Shown {
  ask: ChatAsk;
  after: number;
}

const MARKER = { new_session: "New session", compacted: "Context compacted", session_ended: "Session ended" } as const;
const ended = (at: number) => `${MARKER.session_ended} · ${clock(at)}`;

type EventKind = Extract<ChatBody, { type: "event" }>["kind"];
/** An event's status mark: what needs the owner in the accent, a failure in danger, the rest done. */
export const EVENT_TONE: Record<EventKind, "accent" | "danger" | "done"> = {
  needs_you: "accent",
  checks_failed: "danger",
  checks_passed: "done",
  pr_opened: "done",
  pr_merged: "done",
};

/**
 * An agent's chat, read from Claude's transcript through the daemon: the owner's prompts, Claude's replies, each run
 * of tool calls as one section, and the prompt the agent waits on, answered here. Opens at the latest; earlier
 * history loads as the reader scrolls up.
 */
export function ChatPane(props: Props) {
  // Another agent starts afresh: its own history, scroll and asks.
  return <Chat key={`${props.baseUrl} ${props.agentId}`} {...props} />;
}

function Chat({ baseUrl, agentId, client, factory = browserEventSource }: Props) {
  const [items, setItems] = useState<ChatItem[]>([]);
  // The first page came: an empty chat is then really empty, not still loading.
  const [loaded, setLoaded] = useState(false);
  const [earlier, setEarlier] = useState(0);
  const [asks, setAsks] = useState<Shown[]>([]);
  const [endedAt, setEndedAt] = useState<number | null>(null);
  const [opened, setOpened] = useState<ReadonlyMap<number, boolean>>(new Map());
  const [historyError, setHistoryError] = useState<string | null>(null);
  const feed = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  // The feed's height before an earlier page went in above: the reader is kept on what they were reading.
  const before = useRef<number | null>(null);
  const loading = useRef(false);
  const lastSeq = useRef(-1);
  lastSeq.current = items.at(-1)?.seq ?? -1;

  useEffect(
    () =>
      subscribeChat(
        baseUrl,
        agentId,
        {
          page: (p) => {
            setItems(p.items);
            setEarlier(p.earlier);
            setLoaded(true);
          },
          items: (next) => setItems((prev) => mergeItems(prev, next)),
          ask: (a) =>
            setAsks((list) => {
              // A prompt that cleared unanswered (answered in the terminal, timed out) leaves nothing behind.
              const kept = list.filter((s) => s.ask.answered !== null && s.ask.id !== a?.id);
              if (!a) return kept;
              const known = list.find((s) => s.ask.id === a.id);
              const answered = a.answered ?? known?.ask.answered ?? null;
              return [...kept, { ask: { ...a, answered }, after: answered !== null && known ? known.after : lastSeq.current }];
            }),
          ended: (at) => {
            setEndedAt(at);
            // The stream need not clear an ask before it ends: nothing is left to answer.
            setAsks((list) => list.filter((s) => s.ask.answered !== null));
          },
        },
        factory,
      ),
    [baseUrl, agentId, factory],
  );

  useLayoutEffect(() => {
    const el = feed.current;
    if (!el) return;
    if (before.current !== null) {
      el.scrollTop += el.scrollHeight - before.current;
      before.current = null;
    } else if (atEnd.current) el.scrollTop = el.scrollHeight;
  }, [items, asks, endedAt]);

  const loadEarlier = () => {
    const first = items[0];
    if (loading.current || earlier === 0 || !first) return;
    loading.current = true;
    client
      .chatHistory(agentId, first.seq, PAGE)
      .then(
        (p) => {
          before.current = feed.current?.scrollHeight ?? null;
          setHistoryError(null);
          setItems((prev) => mergeItems(p.items, prev));
          setEarlier(p.earlier);
        },
        (e: unknown) => setHistoryError(e instanceof Error ? e.message : String(e)),
      )
      .finally(() => (loading.current = false));
  };

  const onScroll = () => {
    const el = feed.current!;
    atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight <= AT_END;
    if (el.scrollTop < el.clientHeight) loadEarlier();
  };

  const answered = (id: string, label: string) =>
    setAsks((list) => list.map((s) => (s.ask.id === id ? { ask: { ...s.ask, answered: label }, after: lastSeq.current } : s)));
  const askNode = (s: Shown) => (
    <AskMessage key={`ask ${s.ask.id}`} agentId={agentId} ask={s.ask} client={client} onAnswered={(label) => answered(s.ask.id, label)} />
  );

  // Answered asks go in where they were answered; the one still waiting ends the chat.
  const placed = asks.filter((s) => s.ask.answered !== null);
  const waiting = asks.filter((s) => s.ask.answered === null);
  const rows: ReactNode[] = [];
  let shown = 0;
  for (const block of blocks(items)) {
    const first = block.kind === "item" ? block.item.seq : block.key;
    for (; shown < placed.length && placed[shown]!.after < first; shown++) rows.push(askNode(placed[shown]!));
    if (block.kind === "section") {
      const open = opened.get(block.key) ?? opensItself(block);
      rows.push(<CallSection key={`s ${block.key}`} section={block} open={open} onToggle={() => setOpened((m) => new Map(m).set(block.key, !open))} />);
    } else rows.push(<Entry key={block.item.seq} item={block.item} />);
  }
  rows.push(...placed.slice(shown).map(askNode), ...waiting.map(askNode));
  const endedShown = items.some((i) => i.body.type === "marker" && i.body.kind === "session_ended");
  if (endedAt !== null && !endedShown) rows.push(<p key="ended" className="chat-line" data-marker="session_ended">{ended(endedAt)}</p>);

  return (
    <div className="chat">
      <div ref={feed} role="log" aria-label="Chat" className="chat-feed" onScroll={onScroll}>
        {earlier > 0 && (
          <button type="button" className="chat-earlier" onClick={loadEarlier}>
            ↑ {earlier} earlier {earlier === 1 ? "section" : "sections"}
          </button>
        )}
        {historyError && <ErrorLine text={historyError} />}
        {loaded && rows.length === 0 && <p className="chat-empty">Nothing yet: the chat starts with the agent's first message.</p>}
        {rows}
      </div>
    </div>
  );
}

function Entry({ item }: { item: ChatItem }) {
  const body = item.body;
  switch (body.type) {
    case "prompt":
      // Prompts the owner did not type (a resume, an orchestrator's message) are a small line, not the owner's block.
      return body.from === "owner" ? (
        <div className="chat-prompt" data-from="owner">
          <Markdown text={body.text} />
        </div>
      ) : (
        <p className="chat-line" data-from="system">
          {body.text}
        </p>
      );
    case "reply":
      return (
        <div className="chat-reply">
          <Markdown text={body.text} />
        </div>
      );
    case "marker":
      return (
        <p className="chat-line" data-marker={body.kind}>
          {body.kind === "session_ended" ? ended(item.at) : MARKER[body.kind]}
        </p>
      );
    case "event":
      // A task's event, placed and drawn as a system prompt is.
      return (
        <p className="chat-line" data-from="system" data-event={body.kind}>
          <span className="chat-mark" data-tone={EVENT_TONE[body.kind]} aria-hidden="true" />
          <span className="chat-event-title">{body.task_title}</span> {body.text}
        </p>
      );
    case "call":
      return null; // calls are drawn in their section
  }
}

/** A run of calls: folded, one line with the + / −, the count, the tool names and a mark per call; open, a line each. */
function CallSection({ section, open, onToggle }: { section: Section; open: boolean; onToggle: () => void }) {
  const calls = section.calls.flatMap((c) => (c.body.type === "call" ? [c.body] : []));
  return (
    <div className="chat-section">
      <button type="button" className="chat-section-head" aria-expanded={open} onClick={onToggle}>
        <PlusMinus open={open} />
        <span className="chat-section-name">
          <span className="chat-section-count">
            {calls.length} {calls.length === 1 ? "call" : "calls"}
          </span>
          {` · ${calls.map((c) => c.tool).join(", ")}`}
        </span>
        <span className="chat-marks" aria-hidden="true">
          {calls.map((c) => (
            <span key={c.id} className="chat-mark" data-status={c.status} />
          ))}
        </span>
      </button>
      {open && (
        <div className="chat-calls">
          {calls.map((c) => (
            <CallLine key={c.id} call={c} />
          ))}
        </div>
      )}
    </div>
  );
}

function CallLine({ call }: { call: Call }) {
  return (
    <>
      <div className="chat-call" data-status={call.status}>
        <span className="chat-call-tool">{call.tool}</span>
        <span className="chat-call-target" title={call.target}>
          {call.target}
        </span>
        <span className="chat-call-result">{call.result ?? (call.status === "done" ? "" : call.status)}</span>
      </div>
      {call.status === "failed" && call.output && <pre className="chat-call-output">{call.output}</pre>}
    </>
  );
}
