//! A terminal attached to an agent over a WebSocket: binary frames carry terminal bytes both ways, text frames
//! carry `{"resize":…}` in and `{"exited":…}` out. It sits behind the Host/Origin guard like every route, which is
//! what keeps a web page from opening one.
use crate::session::{Recv, SessionHost, Size, DEFAULT_SIZE};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::time::Instant;

/// How long the start of a possible cursor report is held back waiting for the rest; a lone Escape keypress waits
/// this long.
const HOLD: Duration = Duration::from_millis(50);
/// The longest held prefix: `ESC [` and two five-digit numbers with their `;`.
const MAX_REPORT: usize = 14;

#[derive(serde::Deserialize)]
struct Control {
    resize: Option<Size>,
}

/// Removes cursor-position reports (`ESC [ row ; col R`) from a terminal's input. The daemon answers every cursor
/// query itself; an attached terminal answers it too, and the program must not get two replies.
#[derive(Default)]
pub(crate) struct ReportFilter {
    /// The start of what may be a report, cut off at the end of the last frame.
    held: Vec<u8>,
}

enum Match {
    Report(usize),
    Partial,
    No,
}

/// Whether `b`, which starts with ESC, starts with a whole report, could be the start of one, or is neither.
fn report_at(b: &[u8]) -> Match {
    let mut i = 1;
    if b.len() == i {
        return Match::Partial;
    }
    if b[i] != b'[' {
        return Match::No;
    }
    i += 1;
    for sep in *b";R" {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == b.len() {
            return if i <= MAX_REPORT {
                Match::Partial
            } else {
                Match::No
            };
        }
        if i == start || b[i] != sep {
            return Match::No;
        }
        i += 1;
    }
    Match::Report(i)
}

impl ReportFilter {
    /// `input` without any reports; the start of one cut off at its end is held for the next frame.
    pub(crate) fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        let mut b = std::mem::take(&mut self.held);
        b.extend_from_slice(input);
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] != 0x1b {
                out.push(b[i]);
                i += 1;
                continue;
            }
            match report_at(&b[i..]) {
                Match::Report(n) => i += n,
                Match::Partial => {
                    self.held = b[i..].to_vec();
                    break;
                }
                Match::No => {
                    out.push(b[i]);
                    i += 1;
                }
            }
        }
        out
    }

    /// Whatever is held, given up on: it was not a report after all (a lone Escape, say).
    pub(crate) fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.held)
    }

    pub(crate) fn holding(&self) -> bool {
        !self.held.is_empty()
    }
}

pub(crate) async fn serve(socket: WebSocket, sessions: SessionHost, name: String) {
    let Ok(mut att) = sessions.attach(&name, DEFAULT_SIZE) else {
        return;
    };
    let (mut tx, mut rx) = socket.split();
    if tx
        .send(Message::Binary(att.snapshot().into()))
        .await
        .is_err()
    {
        return;
    }
    let mut filter = ReportFilter::default();
    // When the held start of a report is given up on; `None` while nothing is held.
    let mut give_up: Option<Instant> = None;
    loop {
        tokio::select! {
            out = att.recv() => match out {
                Some(Recv::Bytes(b)) => {
                    if tx.send(Message::Binary(b.to_vec().into())).await.is_err() {
                        return;
                    }
                }
                Some(Recv::Lagged(snapshot)) => {
                    // Clear first: the snapshot redraws every cell, but not the scrolled-off lines a lagged
                    // terminal may still show.
                    let mut redraw = b"\x1b[H\x1b[2J".to_vec();
                    redraw.extend(snapshot);
                    if tx.send(Message::Binary(redraw.into())).await.is_err() {
                        return;
                    }
                }
                Some(Recv::Exited(code)) => {
                    let _ = tx.send(Message::Text(format!(r#"{{"exited":{code}}}"#).into())).await;
                    let _ = tx.close().await;
                    return;
                }
                None => return,
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Binary(b))) => {
                    let keys = filter.filter(&b);
                    give_up = filter.holding().then(|| give_up.unwrap_or_else(|| Instant::now() + HOLD));
                    if !keys.is_empty() && sessions.write_async(&name, keys).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    if let Ok(Control { resize: Some(size) }) = serde_json::from_str(&t) {
                        att.resize(size);
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
            () = tokio::time::sleep_until(give_up.unwrap_or_else(Instant::now)), if give_up.is_some() => {
                give_up = None;
                if sessions.write_async(&name, filter.flush()).await.is_err() {
                    return;
                }
            }
        }
    }
}

pub(crate) fn upgrade(ws: WebSocketUpgrade, sessions: SessionHost, name: String) -> Response {
    ws.on_upgrade(move |socket| serve(socket, sessions, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(frames: &[&[u8]]) -> Vec<u8> {
        let mut f = ReportFilter::default();
        let mut out = Vec::new();
        for fr in frames {
            out.extend(f.filter(fr));
        }
        out.extend(f.flush());
        out
    }

    #[test]
    fn a_report_is_removed() {
        assert_eq!(run(&[b"a\x1b[12;34Rb"]), b"ab");
        assert_eq!(run(&[b"\x1b[1;1R"]), b"");
    }

    #[test]
    fn a_report_split_across_frames_is_removed() {
        for cut in 1..9 {
            let all = b"x\x1b[12;34Ry";
            assert_eq!(run(&[&all[..cut], &all[cut..]]), b"xy", "cut at {cut}");
        }
        let mut f = ReportFilter::default();
        assert_eq!(f.filter(b"x\x1b[12"), b"x");
        assert!(f.holding());
        assert_eq!(f.filter(b";34Ry"), b"y");
        assert!(!f.holding());
    }

    #[test]
    fn text_and_keys_pass_through() {
        let keys: &[&[u8]] = &[
            b"hello world\r",
            b"\x1b[A\x1b[B\x1b[C\x1b[D",
            b"\x1b[1;5C",
            b"\x1b[3~",
            b"\x1bOP",
            b"\x15\x7f",
            b"[12;34R",
            "héllo ✓".as_bytes(),
        ];
        for k in keys {
            assert_eq!(run(&[k]), *k, "{k:?}");
            let mut f = ReportFilter::default();
            assert_eq!(f.filter(k), *k, "{k:?} held back");
        }
    }

    #[test]
    fn a_lone_escape_is_held_then_given_up_on() {
        let mut f = ReportFilter::default();
        assert_eq!(f.filter(b"\x1b"), b"");
        assert!(f.holding());
        assert_eq!(f.flush(), b"\x1b");
        assert_eq!(f.filter(b"\x1b"), b"");
        assert_eq!(f.filter(b"x"), b"\x1bx");
    }

    #[test]
    fn an_endless_number_is_not_held_forever() {
        let long = b"\x1b[1234567890123456";
        assert_eq!(ReportFilter::default().filter(long), long);
    }
}
