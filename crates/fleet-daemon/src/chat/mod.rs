//! Claude Code transcript lines → chat items.
mod build;
mod read;
pub mod stream;
mod summary;

pub use build::Builder;
pub use read::Tail;
pub use summary::{result_of, summarize, OUTPUT_BYTES, OUTPUT_LINES};

use fleet_api::{ChatBody, ChatItem};

/// How many blocks `items` make in the web's grouping: a run of consecutive calls is one, every other item is one.
pub fn blocks_before(items: &[ChatItem]) -> u64 {
    let mut blocks = 0;
    let mut in_calls = false;
    for item in items {
        let is_call = matches!(item.body, ChatBody::Call { .. });
        if !(is_call && in_calls) {
            blocks += 1;
        }
        in_calls = is_call;
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_api::{CallStatus, MarkerKind};

    fn item(body: ChatBody) -> ChatItem {
        ChatItem {
            seq: 0,
            at: 0,
            body,
        }
    }
    fn call() -> ChatItem {
        item(ChatBody::Call {
            id: "t".into(),
            tool: "Read".into(),
            target: String::new(),
            status: CallStatus::Done,
            result: None,
            output: None,
        })
    }
    fn reply() -> ChatItem {
        item(ChatBody::Reply { text: "x".into() })
    }

    #[test]
    fn a_run_of_calls_is_one_block() {
        let items = [
            reply(),
            call(),
            call(),
            call(),
            reply(),
            call(),
            item(ChatBody::Marker {
                kind: MarkerKind::Compacted,
            }),
        ];
        assert_eq!(blocks_before(&items), 5);
        assert_eq!(blocks_before(&[]), 0);
        assert_eq!(blocks_before(&[call(), call()]), 1);
    }
}
