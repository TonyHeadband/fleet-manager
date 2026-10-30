// The footer's message queue: what the app tells the person, one line at a time. A port of Lite's
// crates/fleet-gui/src/messages.rs, rule for rule; times are seconds, passed in so the timing is testable.

/** Seconds a success or info message is shown. */
export const SHOW = 4;
/** Seconds an error is shown: longer, because it is the one that has to be read. */
export const SHOW_ERROR = 8;
/** Messages the history popup keeps. */
export const HISTORY = 10;

export type Notice = "error" | "warning" | "success" | "info";

export interface Message {
  notice: Notice;
  text: string;
  /** Unix seconds, UTC. */
  time: number;
  /** The agent the message is about: clicking the line selects its card. */
  agentId?: string;
}

const same = (a: Message, b: Message) => a.notice === b.notice && a.text === b.text;

export class Messages {
  private queue: Message[] = [];
  private shown: Message | null = null;
  private shownSince = 0;
  // The queue is the cycle rotated to start after `shown`: first the messages not yet shown this cycle, then the
  // `seen` looping ones that already had their turn in it. `i` of `i/n` is `seen + 1`, so it stays on the message on
  // screen when a newcomer joins or a one-shot leaves.
  private seen = 0;
  /** True once `shown` is only what is left on screen after its turn, with nothing queued behind it. */
  private dimmed = false;
  // The last message of this batch that finished its turn without being dismissed: what the footer falls back to,
  // dimmed. A new batch clears it, so a dismiss never brings back a leftover from before.
  private last: Message | null = null;
  private past: Message[] = [];

  constructor(private sticky: boolean) {}

  /** Settings' Footer choice: errors and warnings loop until clicked, or show once like the rest. */
  setSticky(on: boolean) {
    this.sticky = on;
  }

  /** Queues `m`, unless the same message is already waiting or on screen. A dimmed message is only a leftover. */
  push(m: Message, now: number) {
    if (this.queue.some((o) => same(o, m)) || (this.shown && !this.dimmed && same(this.shown, m))) return;
    this.past = [m, ...this.past].slice(0, HISTORY);
    if (this.shown === null || this.dimmed) {
      this.seen = 0;
      this.last = null;
      this.show(m, now);
      return;
    }
    // At the end of this cycle, before the looping messages already shown in it.
    this.queue.splice(this.queue.length - this.seen, 0, m);
    // A lone looping message is not re-shown while nothing ticks, so it may have had its turn long ago.
    const c = this.shown;
    if (this.queue.length === 1 && this.loops(c) && now - this.shownSince >= this.turn(c)) this.advance(now, false);
  }

  /** Moves on once the message on screen has had its turn. */
  tick(now: number) {
    if (this.shown && !this.dimmed && now - this.shownSince >= this.turn(this.shown)) this.advance(now, false);
  }

  /** A click dismisses a message that would otherwise loop, and skips any other. */
  click(now: number) {
    if (this.shown && !this.dimmed) this.advance(now, this.loops(this.shown));
  }

  /** The message on screen and whether it is dimmed (its turn is over and nothing else is waiting). */
  current(): [Message, boolean] | null {
    return this.shown ? [this.shown, this.dimmed] : null;
  }

  /** `[i, n]` while more than one message is waiting for the footer. */
  position(): [number, number] | null {
    const n = this.queue.length + (this.shown && !this.dimmed ? 1 : 0);
    return n > 1 ? [this.seen + 1, n] : null;
  }

  /** Newest first, at most `HISTORY`. */
  history(): Message[] {
    return this.past;
  }

  /** Seconds until the footer next changes by itself, or `null` when it will not. */
  nextChange(now: number): number | null {
    const c = this.dimmed ? null : this.shown;
    if (!c) return null;
    // Alone, a looping message comes straight back as itself.
    if (this.queue.length === 0 && this.loops(c)) return null;
    return Math.max(this.shownSince + this.turn(c) - now, 0);
  }

  private loops(m: Message) {
    return this.sticky && (m.notice === "error" || m.notice === "warning");
  }

  private turn(m: Message) {
    return m.notice === "error" ? SHOW_ERROR : SHOW;
  }

  private show(m: Message, now: number) {
    this.shown = m;
    this.shownSince = now;
    this.dimmed = false;
  }

  private advance(now: number, dismissed: boolean) {
    const done = this.shown;
    if (!done) return;
    this.shown = null;
    if (dismissed) {
      // Never kept as the dimmed leftover: the person asked not to see it again.
    } else if (this.loops(done)) {
      this.queue.push(done);
      this.seen += 1;
    } else {
      this.last = done;
    }
    // Everything left has had its turn in this cycle: the next one starts.
    if (this.queue.length === this.seen) this.seen = 0;
    const next = this.queue.shift();
    if (next) this.show(next, now);
    else {
      this.shown = this.last;
      this.dimmed = true;
    }
  }
}
