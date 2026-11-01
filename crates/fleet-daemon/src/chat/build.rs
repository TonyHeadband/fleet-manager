use crate::agents::{PROMPT_POINTER, RESUME_PROMPT};
use crate::chat::summary::{result_of, summarize};
use crate::merges::parse_timestamp;
use fleet_api::{CallStatus, ChatBody, ChatItem, MarkerKind, PromptFrom};
use serde_json::Value;
use std::collections::HashMap;

/// A `tool_use` still waiting for its `tool_result`.
struct OpenCall {
    seq: u64,
    at: i64,
    raw_tool: String,
    tool: String,
    target: String,
    input: Value,
}

/// Turns transcript lines into chat items, minting each item's `seq`.
pub struct Builder {
    next_seq: u64,
    last_at: i64,
    prompt_text: Option<String>,
    open: HashMap<String, OpenCall>,
}

impl Builder {
    /// `prompt_text` is the task's PROMPT.md, shown in place of the pointer line the agent was started with.
    pub fn new(prompt_text: Option<String>) -> Self {
        Self {
            next_seq: 0,
            last_at: 0,
            prompt_text,
            open: HashMap::new(),
        }
    }

    /// One transcript line → the items it adds or updates (an updated call keeps its seq).
    pub fn line(&mut self, line: &str) -> Vec<ChatItem> {
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        if let Some(at) = rec
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            self.last_at = at;
        }
        if rec.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return Vec::new();
        }
        let content = &rec["message"]["content"];
        match rec.get("type").and_then(Value::as_str) {
            Some("user") => self.user(&rec, content),
            Some("assistant") => self.assistant(content),
            Some("system") if rec["subtype"] == "compact_boundary" => {
                vec![self.marker(MarkerKind::Compacted, self.last_at)]
            }
            _ => Vec::new(),
        }
    }

    pub fn marker(&mut self, kind: MarkerKind, at: i64) -> ChatItem {
        self.mint(at, ChatBody::Marker { kind })
    }

    pub fn new_session(&mut self, at: i64) -> ChatItem {
        self.marker(MarkerKind::NewSession, at)
    }

    fn mint(&mut self, at: i64, body: ChatBody) -> ChatItem {
        let seq = self.next_seq;
        self.next_seq += 1;
        ChatItem { seq, at, body }
    }

    fn user(&mut self, rec: &Value, content: &Value) -> Vec<ChatItem> {
        let internal = rec.get("isMeta").and_then(Value::as_bool) == Some(true)
            || rec.get("isCompactSummary").and_then(Value::as_bool) == Some(true);
        if internal {
            return Vec::new();
        }
        match content {
            Value::String(text) => self.prompt(text).into_iter().collect(),
            Value::Array(blocks) => {
                let mut items = Vec::new();
                for block in blocks {
                    match block["type"].as_str() {
                        Some("tool_result") => items.extend(self.finish_call(block)),
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or_default();
                            items.extend(self.prompt(text));
                        }
                        _ => {}
                    }
                }
                items
            }
            _ => Vec::new(),
        }
    }

    /// The prompt a user line shows, or None for a slash command and the output Claude Code records around one.
    fn prompt(&mut self, text: &str) -> Option<ChatItem> {
        let trimmed = text.trim();
        let command = trimmed.starts_with("<command-")
            || trimmed.starts_with("<local-command")
            || (trimmed.starts_with('/') && !trimmed.contains(char::is_whitespace));
        if trimmed.is_empty() || command {
            return None;
        }
        let (text, from) = if text == PROMPT_POINTER {
            (
                self.prompt_text.clone().unwrap_or_else(|| text.to_string()),
                PromptFrom::Owner,
            )
        } else if text == RESUME_PROMPT {
            (text.to_string(), PromptFrom::System)
        } else {
            (text.to_string(), PromptFrom::Owner)
        };
        Some(self.mint(self.last_at, ChatBody::Prompt { text, from }))
    }

    fn assistant(&mut self, content: &Value) -> Vec<ChatItem> {
        let Some(blocks) = content.as_array() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default();
                    if !text.trim().is_empty() {
                        let body = ChatBody::Reply {
                            text: text.to_string(),
                        };
                        items.push(self.mint(self.last_at, body));
                    }
                }
                Some("tool_use") => items.extend(self.start_call(block)),
                _ => {}
            }
        }
        items
    }

    fn start_call(&mut self, block: &Value) -> Option<ChatItem> {
        let id = block["id"].as_str()?.to_string();
        let raw_tool = block["name"].as_str()?.to_string();
        let input = block["input"].clone();
        let tool = display_name(&raw_tool);
        let target = summarize(&raw_tool, &input);
        let body = ChatBody::Call {
            id: id.clone(),
            tool: tool.clone(),
            target: target.clone(),
            status: CallStatus::Running,
            result: None,
            output: None,
        };
        let item = self.mint(self.last_at, body);
        self.open.insert(
            id,
            OpenCall {
                seq: item.seq,
                at: item.at,
                raw_tool,
                tool,
                target,
                input,
            },
        );
        Some(item)
    }

    fn finish_call(&mut self, block: &Value) -> Option<ChatItem> {
        let id = block["tool_use_id"].as_str()?;
        let call = self.open.remove(id)?;
        let is_error = block["is_error"].as_bool().unwrap_or(false);
        let (status, result, output) =
            result_of(&call.raw_tool, &call.input, &block["content"], is_error);
        Some(ChatItem {
            seq: call.seq,
            at: call.at,
            body: ChatBody::Call {
                id: id.to_string(),
                tool: call.tool,
                target: call.target,
                status,
                result,
                output,
            },
        })
    }
}

/// `mcp__fleet__finish_step` shows as `finish_step`.
fn display_name(raw: &str) -> String {
    raw.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map_or(raw, |(_, tool)| tool)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{PROMPT_POINTER, RESUME_PROMPT};
    use fleet_api::{CallStatus, ChatBody, PromptFrom};
    use serde_json::json;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/transcripts/{name}.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn run(b: &mut Builder, text: &str) -> Vec<ChatItem> {
        text.lines().flat_map(|l| b.line(l)).collect()
    }

    fn user(content: serde_json::Value) -> String {
        json!({"type": "user", "timestamp": "2026-11-01T03:27:24.604Z", "message": {"role": "user", "content": content}})
            .to_string()
    }

    fn call_parts(i: &ChatItem) -> (&str, &str, CallStatus, Option<&str>) {
        match &i.body {
            ChatBody::Call {
                tool,
                target,
                status,
                result,
                ..
            } => (tool, target, *status, result.as_deref()),
            other => panic!("not a call: {other:?}"),
        }
    }

    #[test]
    fn the_first_prompt_shows_prompt_md() {
        let mut b = Builder::new(Some("Fix the bug.".into()));
        let items = b.line(&user(json!(PROMPT_POINTER)));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].at, 1_793_503_644);
        assert_eq!(
            items[0].body,
            ChatBody::Prompt {
                text: "Fix the bug.".into(),
                from: PromptFrom::Owner
            }
        );
    }

    #[test]
    fn system_prompts_are_marked() {
        let mut b = Builder::new(None);
        let items = b.line(&user(json!(RESUME_PROMPT)));
        assert_eq!(
            items[0].body,
            ChatBody::Prompt {
                text: RESUME_PROMPT.into(),
                from: PromptFrom::System
            }
        );
    }

    #[test]
    fn typed_prompts_are_the_owners() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("tools"));
        match &items[0].body {
            ChatBody::Prompt { text, from } => {
                assert!(text.starts_with("Do these steps"));
                assert_eq!(*from, PromptFrom::Owner);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tool_use_is_running_until_its_result() {
        let text = fixture("tools");
        let mut b = Builder::new(None);
        let mut lines = text.lines();
        let mut running = None;
        for l in lines.by_ref() {
            let items = b.line(l);
            if let Some(i) = items
                .iter()
                .find(|i| matches!(i.body, ChatBody::Call { .. }))
            {
                running = Some(i.clone());
                break;
            }
        }
        let running = running.unwrap();
        assert_eq!(call_parts(&running).2, CallStatus::Running);
        let updated = lines
            .flat_map(|l| b.line(l))
            .find(|i| i.seq == running.seq)
            .unwrap();
        assert_eq!(updated.seq, running.seq);
        assert_eq!(updated.at, running.at);
        assert_eq!(call_parts(&updated).2, CallStatus::Done);
    }

    #[test]
    fn targets_and_results_per_tool() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("tools"));
        let calls: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.body, ChatBody::Call { .. }))
            .collect();
        let finished: Vec<_> = calls
            .iter()
            .filter(|i| call_parts(i).2 != CallStatus::Running)
            .collect();
        let read = call_parts(finished[0]);
        assert_eq!(
            read,
            (
                "Read",
                "/tmp/fixture/tools/hello.py",
                CallStatus::Done,
                Some("4 lines")
            )
        );
        let bash = call_parts(finished[1]);
        assert_eq!(bash.0, "Bash");
        assert!(bash.1.starts_with("grep -rn"));
        let edit = call_parts(finished[2]);
        assert_eq!(
            edit,
            (
                "Edit",
                "/tmp/fixture/tools/hello.py",
                CallStatus::Done,
                Some("+2 \u{2212}1")
            )
        );
        let failed = call_parts(finished[3]);
        assert_eq!(failed.2, CallStatus::Failed);
        assert_eq!(failed.3, Some("Exit code 1"));
        assert_eq!(finished.len(), 4);
    }

    #[test]
    fn grep_and_glob_summaries_follow_the_tool_use_block() {
        let mut b = Builder::new(None);
        let use_line = |id: &str, name: &str, input: serde_json::Value| {
            json!({"type": "assistant", "timestamp": "2026-11-01T03:27:24.604Z", "message": {"content": [
                {"type": "tool_use", "id": id, "name": name, "input": input}]}})
            .to_string()
        };
        let result_line = |id: &str, text: &str| {
            user(json!([{"type": "tool_result", "tool_use_id": id, "content": text}]))
        };
        b.line(&use_line("g1", "Grep", json!({"pattern": "TODO"})));
        let g = &b.line(&result_line("g1", "a.rs\nb.rs"))[0];
        assert_eq!(
            call_parts(g),
            ("Grep", "TODO", CallStatus::Done, Some("2 files"))
        );
        b.line(&use_line("g2", "Glob", json!({"pattern": "*.rs"})));
        let g = &b.line(&result_line("g2", "a.rs"))[0];
        assert_eq!(
            call_parts(g),
            ("Glob", "*.rs", CallStatus::Done, Some("1 file"))
        );
    }

    #[test]
    fn mcp_tools_drop_their_server_prefix() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("mcp"));
        let tools: Vec<_> = items
            .iter()
            .filter_map(|i| match &i.body {
                ChatBody::Call { tool, .. } => Some(tool.as_str()),
                _ => None,
            })
            .collect();
        assert!(tools.contains(&"add"), "{tools:?}");
        let line = json!({"type": "assistant", "timestamp": "2026-11-01T03:27:24.604Z", "message": {"content": [
            {"type": "tool_use", "id": "f", "name": "mcp__fleet__finish_step", "input": {}}]}})
        .to_string();
        let items = Builder::new(None).line(&line);
        assert_eq!(call_parts(&items[0]).0, "finish_step");
    }

    #[test]
    fn a_denied_call_fails() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("permission"));
        assert!(items.iter().any(|i| matches!(
            i.body,
            ChatBody::Call {
                status: CallStatus::Failed,
                ..
            }
        )));
    }

    #[test]
    fn compaction_is_one_marker() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("compact"));
        let markers: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.body, ChatBody::Marker { .. }))
            .collect();
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].body,
            ChatBody::Marker {
                kind: MarkerKind::Compacted
            }
        );
        let prompts: Vec<_> = items
            .iter()
            .filter_map(|i| match &i.body {
                ChatBody::Prompt { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            prompts,
            [
                "Read notes.txt and tell me its first word.",
                "What was the first word of notes.txt? Answer from memory, no tools."
            ]
        );
    }

    #[test]
    fn clear_commands_add_nothing() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("clear-after"));
        assert!(
            matches!(&items[0].body, ChatBody::Prompt { text, .. } if text.starts_with("Read notes.txt"))
        );
    }

    #[test]
    fn unknown_records_are_skipped() {
        let mut b = Builder::new(None);
        assert!(b.line(r#"{"type":"brand-new-thing","x":1}"#).is_empty());
        assert!(b.line("not json").is_empty());
        let odd = json!({"type": "assistant", "timestamp": "2026-11-01T03:27:24.604Z",
            "message": {"content": [{"type": "hologram", "x": 1}, {"type": "thinking", "thinking": ""}]}})
        .to_string();
        assert!(b.line(&odd).is_empty());
        let odd = user(json!([{"type": "hologram"}]));
        assert!(b.line(&odd).is_empty());
    }

    #[test]
    fn seqs_rise_across_items_and_markers() {
        let mut b = Builder::new(None);
        let a = b.new_session(5);
        let c = b.marker(MarkerKind::SessionEnded, 6);
        let p = b.line(&user(json!("hi")));
        assert_eq!((a.seq, c.seq, p[0].seq), (0, 1, 2));
        assert_eq!(
            a.body,
            ChatBody::Marker {
                kind: MarkerKind::NewSession
            }
        );
    }

    #[test]
    fn replies_carry_text() {
        let mut b = Builder::new(None);
        let items = run(&mut b, &fixture("mcp"));
        assert!(items
            .iter()
            .any(|i| i.body == ChatBody::Reply { text: "42".into() }));
    }
}
