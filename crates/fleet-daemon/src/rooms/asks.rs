//! The held `room_ask` calls, by post id, as `breakdown_waiters` holds `propose_breakdown`'s.
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Default)]
pub struct Asks {
    /// Held from storing an ask until its call is registered, and from storing an answer until it is handed on:
    /// an answer can never land in between and reach the asker twice, held and typed.
    pub(crate) lock: tokio::sync::Mutex<()>,
    held: Mutex<HashMap<i64, oneshot::Sender<String>>>,
}

impl Asks {
    pub fn hold(&self, ask: i64) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        self.held.lock().expect("asks poisoned").insert(ask, tx);
        rx
    }

    /// Hands the answer to the held call; false when there is none any more (timed out, hung up, restarted).
    pub fn resolve(&self, ask: i64, answer: &str) -> bool {
        let tx = self.held.lock().expect("asks poisoned").remove(&ask);
        tx.is_some_and(|tx| tx.send(answer.to_string()).is_ok())
    }

    /// Drops the call whose hold ran out.
    pub fn forget(&self, ask: i64) {
        self.held.lock().expect("asks poisoned").remove(&ask);
    }
}
