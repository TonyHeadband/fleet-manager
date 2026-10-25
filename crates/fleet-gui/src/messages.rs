//! The footer's message queue: what the app tells the user, one line at a time. No egui here; the caller passes
//! time in seconds so the timing is testable without a window.
use fleet_log::Notice;
use std::collections::VecDeque;

/// Seconds a success or info message is shown.
pub const SHOW: f64 = 4.0;
/// Seconds an error or warning is shown: longer, because it is the one that has to be read.
pub const SHOW_ERROR: f64 = 8.0;
/// Messages the history popup keeps.
pub const HISTORY: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub notice: Notice,
    pub text: String,
    /// Unix seconds, UTC, as the log entry carries it.
    pub time: i64,
    /// The agent the message is about, when the notice named one: clicking the line selects its card.
    pub agent_id: Option<String>,
}

pub struct Messages {
    queue: VecDeque<Message>,
    current: Option<Message>,
    shown_since: f64,
    /// The queue is the cycle rotated to start after `current`: first the messages not yet shown this cycle, then
    /// the `seen` looping ones that already had their turn in it. `i` of `i/n` is `seen + 1`, so it stays on the
    /// message on screen when a newcomer joins or a one-shot leaves.
    seen: usize,
    /// True once `current` is only what is left on screen after its turn, with nothing queued behind it.
    dimmed: bool,
    /// The last message of this batch that finished its turn without being dismissed: what the footer falls back
    /// to, dimmed. A new batch clears it, so a dismiss never brings back a leftover from before.
    last: Option<Message>,
    history: Vec<Message>,
    sticky: bool,
}

impl Messages {
    pub fn new(sticky: bool) -> Messages {
        Messages {
            queue: VecDeque::new(),
            current: None,
            seen: 0,
            shown_since: 0.0,
            dimmed: false,
            last: None,
            history: Vec::new(),
            sticky,
        }
    }

    pub fn set_sticky(&mut self, on: bool) {
        self.sticky = on;
    }

    /// Queues `m`, unless the same message is already waiting or on screen. A dimmed message is only a leftover,
    /// so it does not count: the same text again is news.
    pub fn push(&mut self, m: Message, now: f64) {
        let same = |o: &Message| o.notice == m.notice && o.text == m.text;
        if self.queue.iter().any(same)
            || self
                .current
                .as_ref()
                .is_some_and(|c| !self.dimmed && same(c))
        {
            return;
        }
        self.history.insert(0, m.clone());
        self.history.truncate(HISTORY);
        if self.current.is_none() || self.dimmed {
            self.seen = 0;
            self.last = None;
            self.show(m, now);
            return;
        }
        // At the end of this cycle, before the looping messages already shown in it.
        self.queue.insert(self.queue.len() - self.seen, m);
        // A lone looping message is not re-shown while the window sleeps, so it may have had its turn long ago.
        if let Some(c) = self
            .current
            .as_ref()
            .filter(|c| self.queue.len() == 1 && self.loops(c))
        {
            if now - self.shown_since >= self.turn(c) {
                self.advance(now, false);
            }
        }
    }

    /// Moves on once the current message has had its turn.
    pub fn tick(&mut self, now: f64) {
        if let Some(c) = &self.current {
            if !self.dimmed && now - self.shown_since >= self.turn(c) {
                self.advance(now, false);
            }
        }
    }

    /// A click dismisses a message that would otherwise loop, and skips any other.
    pub fn click(&mut self, now: f64) {
        if let Some(c) = &self.current {
            if !self.dimmed {
                let dismiss = self.loops(c);
                self.advance(now, dismiss);
            }
        }
    }

    /// The message on screen and whether it is dimmed (its turn is over and nothing else is waiting).
    pub fn current(&self) -> Option<(&Message, bool)> {
        self.current.as_ref().map(|m| (m, self.dimmed))
    }

    /// `(i, n)` while more than one message is waiting for the footer: `n` messages in the cycle, `i` the one on screen.
    pub fn position(&self) -> Option<(usize, usize)> {
        let n = self.queue.len() + usize::from(self.current.is_some() && !self.dimmed);
        (n > 1).then_some((self.seen + 1, n))
    }

    /// Newest first, at most `HISTORY`.
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Seconds until the footer next changes by itself, so the window can sleep until then. `None` when it will not.
    pub fn next_change(&self, now: f64) -> Option<f64> {
        let c = self.current.as_ref().filter(|_| !self.dimmed)?;
        // Alone, a looping message comes straight back as itself.
        if self.queue.is_empty() && self.loops(c) {
            return None;
        }
        Some((self.shown_since + self.turn(c) - now).max(0.0))
    }

    fn loops(&self, m: &Message) -> bool {
        self.sticky && matches!(m.notice, Notice::Error | Notice::Warning)
    }

    fn turn(&self, m: &Message) -> f64 {
        match m.notice {
            Notice::Error => SHOW_ERROR,
            Notice::Warning | Notice::Success | Notice::Info => SHOW,
        }
    }

    fn show(&mut self, m: Message, now: f64) {
        self.current = Some(m);
        self.shown_since = now;
        self.dimmed = false;
    }

    fn advance(&mut self, now: f64, dismissed: bool) {
        let Some(done) = self.current.take() else {
            return;
        };
        if dismissed {
            // Never kept as the dimmed leftover: the user asked not to see it again.
        } else if self.loops(&done) {
            self.queue.push_back(done);
            self.seen += 1;
        } else {
            self.last = Some(done);
        }
        if self.queue.len() == self.seen {
            // Everything left has had its turn in this cycle: the next one starts.
            self.seen = 0;
        }
        match self.queue.pop_front() {
            Some(next) => self.show(next, now),
            None => {
                self.current = self.last.clone();
                self.dimmed = true;
            }
        }
    }
}
