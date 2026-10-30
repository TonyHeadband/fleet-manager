import { useCallback, useEffect, useRef, useState } from "react";
import { Messages, type Message, type Notice } from "./messages";

interface Props {
  messages: Messages;
  /** The line was clicked: it is dismissed or skipped. */
  onClick: () => void;
  /** A message about an agent was clicked: its card is selected. */
  onSelect: (agentId: string) => void;
}

/** `HH:MM`, UTC like Lite's footer, which reads it from the log. */
function clock(unix: number) {
  const secs = ((unix % 86_400) + 86_400) % 86_400;
  return `${String(Math.floor(secs / 3600)).padStart(2, "0")}:${String(Math.floor(secs / 60) % 60).padStart(2, "0")}`;
}

/** One row tall, so a newline would not fit. */
const oneLine = (text: string) => text.replace(/[\r\n]/g, " ");

const Dot = ({ notice }: { notice: Notice }) => <span className="dot" data-notice={notice} aria-hidden="true" />;

/** The content footer: one message line, cycling through the queue, and a History popup. A port of Lite's footer.rs. */
export function Footer({ messages, onClick, onSelect }: Props) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  // As Lite's popup: a press anywhere else closes it, and so does Escape, which hands the focus back to History.
  useEffect(() => {
    if (!open) return;
    const press = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      toggle.current?.focus();
    };
    document.addEventListener("pointerdown", press);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", press);
      document.removeEventListener("keydown", key);
    };
  }, [open]);
  const current = messages.current();
  const position = messages.position();
  const history = messages.history();

  const line = (message: Message, dimmed: boolean) => {
    const text = oneLine(message.text);
    // A dimmed message is only a leftover: clicking it does nothing, so it is not a button. One about an agent still selects its card.
    if (dimmed && !message.agentId) {
      return (
        <span className="footer-text" data-dimmed="true" title={message.text}>
          {text}
        </span>
      );
    }
    return (
      <button
        type="button"
        className="footer-text"
        data-dimmed={dimmed || undefined}
        title={message.text}
        onClick={() => {
          if (message.agentId) onSelect(message.agentId);
          onClick();
        }}
      >
        {text}
      </button>
    );
  };

  return (
    <footer className="footer">
      <div className="footer-line">
        {current && (
          <>
            <Dot notice={current[0].notice} />
            {line(current[0], current[1])}
          </>
        )}
      </div>
      {position && <span className="footer-position">{`${position[0]}/${position[1]}`}</span>}
      {history.length > 0 && (
        <div className="footer-history" ref={box}>
          <button type="button" ref={toggle} className="ghost" aria-expanded={open} onClick={() => setOpen(!open)}>
            History
          </button>
          {open && (
            <div className="history-popup">
              <ul>
                {history.map((m, n) => (
                  <li key={n}>
                    <span className="mono" title="UTC">
                      {clock(m.time)}
                    </span>
                    <Dot notice={m.notice} />
                    <span title={m.text}>{oneLine(m.text)}</span>
                  </li>
                ))}
              </ul>
              <button type="button" className="ghost" disabled title="not on the web yet">
                Open log →
              </button>
            </div>
          )}
        </div>
      )}
    </footer>
  );
}

const queueClock = () => performance.now() / 1000;

/** The page's message queue. `say` queues a line; the footer re-renders whenever the queue moves on by itself. */
export function useMessages() {
  // Errors and warnings loop until dismissed: Lite's default for its "sticky alerts" setting, which the web gets with Settings.
  const messages = useRef(new Messages(true)).current;
  const [, setVersion] = useState(0);
  const changed = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    const wait = messages.nextChange(queueClock());
    if (wait === null) return;
    // The small margin keeps the turn from landing a hair early, as in Lite.
    const t = setTimeout(() => {
      messages.tick(queueClock());
      changed();
    }, (wait + 0.05) * 1000);
    return () => clearTimeout(t);
  });

  const say = useCallback(
    (notice: Notice, text: string, agentId?: string) => {
      messages.push({ notice, text, time: Math.floor(Date.now() / 1000), agentId }, queueClock());
      changed();
    },
    [messages, changed],
  );
  const click = useCallback(() => {
    messages.click(queueClock());
    changed();
  }, [messages, changed]);
  return { messages, say, click };
}
