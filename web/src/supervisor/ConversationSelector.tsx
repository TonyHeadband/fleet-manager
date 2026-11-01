import type { Conversation } from "../api/generated/Conversation";
import type { Status } from "../api/generated/Status";
import type { Tone } from "../api/generated/Tone";
import { Button } from "../ui/Button";
import { FilterMenu } from "../ui/FilterMenu";

/** The board's column tones for each status, so a mark reads as its card does. */
const TONE: Record<Status, Tone> = {
  queued: "neutral",
  working: "active",
  talking: "active",
  needs_you: "attention",
  review: "done",
  stopped: "muted",
};

const dayStart = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** `today`, `yesterday`, then `Oct 30`. */
export function dayLabel(seconds: number, nowSeconds: number): string {
  const days = Math.round((dayStart(new Date(nowSeconds * 1000)) - dayStart(new Date(seconds * 1000))) / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  return new Date(seconds * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export const titleOf = (c: Conversation) => c.title || "Untitled";

interface Props {
  conversations: Conversation[];
  current: string | null;
  onPick(id: string): void;
  onNew(): void;
}

/** The chat header's picker: the persona's conversations under the current one's title, and `+ New`. */
export function ConversationSelector({ conversations, current, onPick, onNew }: Props) {
  const now = Date.now() / 1000;
  const picked = conversations.find((c) => c.id === current);
  return (
    <div className="conversation-selector">
      <FilterMenu label="Conversation" name="Conversation" summary={picked ? titleOf(picked) : "New conversation"} width="release">
        {(close) =>
          conversations.map((c) => (
            <button
              key={c.id}
              type="button"
              className="menu-item conversation-row"
              aria-pressed={c.id === current}
              onClick={() => {
                close();
                if (c.id !== current) onPick(c.id);
              }}
            >
              <span className="conversation-title">{titleOf(c)}</span>
              <span className="conversation-date">{dayLabel(c.last_used_at, now)}</span>
              <span className="conversation-marks" aria-hidden="true">
                {c.tasks.map((t) => (
                  <span key={t.id} className="dot" data-tone={TONE[t.status]} />
                ))}
              </span>
            </button>
          ))
        }
      </FilterMenu>
      <Button onClick={onNew}>+ New</Button>
    </div>
  );
}
