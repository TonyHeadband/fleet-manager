import type { Event } from "./generated/Event";
import type { Snapshot } from "./generated/Snapshot";
import type { ChatAsk } from "./generated/ChatAsk";
import type { ChatItem } from "./generated/ChatItem";
import type { ChatPage } from "./generated/ChatPage";

export type EventSourceFactory = (url: string) => EventSource;
export const browserEventSource: EventSourceFactory = (url) => new EventSource(url);

function parse<T>(m: MessageEvent, sink: (v: T) => void) {
  try {
    sink(JSON.parse(m.data as string) as T);
  } catch {
    // A malformed frame must not take the page down; the next snapshot repairs any gap.
  }
}

/** The daemon pings every 15 s; this long with nothing at all means the connection is dead even if still open. */
export const SILENCE_MS = 45_000;

/**
 * Live board feed. The browser reconnects by itself and resends Last-Event-ID, and the daemon answers
 * with the missed events or a fresh snapshot, so the caller only has to apply what arrives. A connection that
 * stays open but silent (a suspended daemon, a dead link the browser has not noticed) is closed and opened again,
 * which brings a fresh snapshot.
 */
export function subscribe(
  baseUrl: string,
  onSnapshot: (s: Snapshot) => void,
  onEvent: (e: Event) => void,
  factory: EventSourceFactory = browserEventSource,
  onConnection?: (connected: boolean) => void,
  /** The stream opened again after it had been down; not called on the first open. */
  onReconnect?: () => void,
): () => void {
  let es: EventSource;
  let opened = false;
  let lastHeard = Date.now();
  const heard = () => {
    lastHeard = Date.now();
  };
  const open = () => {
    lastHeard = Date.now();
    es = factory(`${baseUrl}/api/events`);
    es.addEventListener("snapshot", (m) => {
      heard();
      parse<Snapshot>(m as MessageEvent, onSnapshot);
    });
    es.addEventListener("event", (m) => {
      heard();
      parse<Event>(m as MessageEvent, onEvent);
    });
    es.addEventListener("ping", heard);
    es.addEventListener("open", () => {
      heard();
      onConnection?.(true);
      // The browser reopens a dropped stream itself and the watchdog opens a new one: either way, a second open.
      if (opened) onReconnect?.();
      opened = true;
    });
    es.addEventListener("error", () => onConnection?.(false));
  };
  open();
  const watchdog = setInterval(() => {
    if (Date.now() - lastHeard >= SILENCE_MS) {
      es.close();
      onConnection?.(false);
      open();
    }
  }, 5_000);
  return () => {
    clearInterval(watchdog);
    es.close();
  };
}

/** Read-only pane text for one agent. */
export function subscribePane(
  baseUrl: string,
  agentId: string,
  onText: (text: string) => void,
  onClosed: () => void,
  factory: EventSourceFactory = browserEventSource,
): () => void {
  const es = factory(`${baseUrl}/api/agents/${encodeURIComponent(agentId)}/terminal`);
  es.addEventListener("pane", (m) => parse<string>(m as MessageEvent, onText));
  es.addEventListener("closed", () => {
    es.close(); // otherwise the browser would reconnect to a session that is gone
    onClosed();
  });
  return () => es.close();
}

/** One agent's conversation: the newest page first, then items by `seq` and the open ask as they change. */
export function subscribeChat(
  baseUrl: string,
  agentId: string,
  on: {
    page(p: ChatPage): void;
    items(i: ChatItem[]): void;
    ask(a: ChatAsk | null): void;
    ended(at: number): void;
  },
  factory: EventSourceFactory = browserEventSource,
): () => void {
  const es = factory(`${baseUrl}/api/agents/${encodeURIComponent(agentId)}/chat`);
  es.addEventListener("page", (m) => parse<ChatPage>(m as MessageEvent, on.page));
  es.addEventListener("items", (m) => parse<ChatItem[]>(m as MessageEvent, on.items));
  es.addEventListener("ask", (m) => parse<ChatAsk | null>(m as MessageEvent, on.ask));
  es.addEventListener("ended", (m) => {
    es.close(); // the daemon is done with this chat; a reconnect would only replay it
    parse<number>(m as MessageEvent, on.ended);
  });
  return () => es.close();
}
