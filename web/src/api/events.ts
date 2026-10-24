import type { Event } from "./generated/Event";
import type { Snapshot } from "./generated/Snapshot";

export type EventSourceFactory = (url: string) => EventSource;
export const browserEventSource: EventSourceFactory = (url) => new EventSource(url);

function parse<T>(m: MessageEvent, sink: (v: T) => void) {
  try {
    sink(JSON.parse(m.data as string) as T);
  } catch {
    // A malformed frame must not take the page down; the next snapshot repairs any gap.
  }
}

/**
 * Live board feed. The browser reconnects by itself and resends Last-Event-ID, and the daemon answers
 * with the missed events or a fresh snapshot, so the caller only has to apply what arrives.
 */
export function subscribe(
  baseUrl: string,
  onSnapshot: (s: Snapshot) => void,
  onEvent: (e: Event) => void,
  factory: EventSourceFactory = browserEventSource,
  onConnection?: (connected: boolean) => void,
): () => void {
  const es = factory(`${baseUrl}/api/events`);
  es.addEventListener("snapshot", (m) => parse<Snapshot>(m as MessageEvent, onSnapshot));
  es.addEventListener("event", (m) => parse<Event>(m as MessageEvent, onEvent));
  es.addEventListener("open", () => onConnection?.(true));
  es.addEventListener("error", () => onConnection?.(false));
  return () => es.close();
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
