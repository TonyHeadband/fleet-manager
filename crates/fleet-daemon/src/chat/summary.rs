use fleet_api::CallStatus;
use serde_json::Value;

pub const OUTPUT_LINES: usize = 40;
pub const OUTPUT_BYTES: usize = 8 * 1024;
const MAX_TARGET_CHARS: usize = 200;

/// What a tool call acts on, for the chat's one-line view of it.
pub fn summarize(tool: &str, input: &Value) -> String {
    let key = match tool {
        "Read" | "Edit" | "Write" | "MultiEdit" => "file_path",
        "NotebookEdit" => "notebook_path",
        "Bash" => "command",
        "Grep" | "Glob" => "pattern",
        "WebFetch" => "url",
        "WebSearch" | "ToolSearch" => "query",
        "Agent" | "Task" => "description",
        "Skill" => "skill",
        _ => return String::new(),
    };
    let target = input.get(key).and_then(Value::as_str).unwrap_or_default();
    target.chars().take(MAX_TARGET_CHARS).collect()
}

/// The status, short result line and (for a failure) capped output of a finished call. `result` is the
/// `tool_result` block's `content`: a string or a list of blocks.
pub fn result_of(
    tool: &str,
    input: &Value,
    result: &Value,
    is_error: bool,
) -> (CallStatus, Option<String>, Option<String>) {
    let text = result_text(result);
    if is_error {
        let first = text
            .lines()
            .next()
            .filter(|l| !l.is_empty())
            .map(str::to_string);
        return (
            CallStatus::Failed,
            first,
            Some(cap(&text)).filter(|o| !o.is_empty()),
        );
    }
    let line = match tool {
        "Read" => Some(count(read_lines(&text), "line")),
        "Edit" => Some(edit_delta(input)),
        "Grep" | "Glob" => Some(count(files_listed(&text), "file")),
        _ => None,
    };
    (CallStatus::Done, line, None)
}

fn result_text(result: &Value) -> String {
    match result {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// Read's output is `cat -n` of the file, and a file ending in a newline gets an empty numbered line after it.
fn read_lines(text: &str) -> usize {
    let mut lines: Vec<&str> = text.lines().collect();
    while lines
        .last()
        .is_some_and(|l| l.split_once('\t').is_some_and(|(_, rest)| rest.is_empty()))
    {
        lines.pop();
    }
    lines.len()
}

fn files_listed(text: &str) -> usize {
    if text.trim() == "No files found" {
        return 0;
    }
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// `+N −M` for an edit: its lines added and removed, without the lines the old and new text share at either end.
fn edit_delta(input: &Value) -> String {
    let lines = |key: &str| -> Vec<&str> {
        input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .collect()
    };
    let (old, new) = (lines("old_string"), lines("new_string"));
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    format!(
        "+{} \u{2212}{}",
        new.len() - head - tail,
        old.len() - head - tail
    )
}

/// The first `OUTPUT_LINES` lines, cut to `OUTPUT_BYTES` on a character boundary.
fn cap(text: &str) -> String {
    let mut out = text
        .lines()
        .take(OUTPUT_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if out.len() > OUTPUT_BYTES {
        let mut end = OUTPUT_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn done(tool: &str, input: Value, result: Value) -> Option<String> {
        let (status, line, _) = result_of(tool, &input, &result, false);
        assert_eq!(status, CallStatus::Done);
        line
    }

    #[test]
    fn targets_per_tool() {
        assert_eq!(
            summarize("Read", &json!({"file_path": "/a/b.rs"})),
            "/a/b.rs"
        );
        assert_eq!(
            summarize("Edit", &json!({"file_path": "/a/b.rs", "old_string": "x"})),
            "/a/b.rs"
        );
        assert_eq!(
            summarize("Bash", &json!({"command": "ls -l", "description": "d"})),
            "ls -l"
        );
        assert_eq!(
            summarize("Grep", &json!({"pattern": "TODO", "path": "."})),
            "TODO"
        );
        assert_eq!(summarize("Glob", &json!({"pattern": "**/*.rs"})), "**/*.rs");
        assert_eq!(summarize("mcp__calc__add", &json!({"a": 1, "b": 2})), "");
        assert_eq!(summarize("Read", &json!(null)), "");
    }

    #[test]
    fn a_long_target_is_cut() {
        let long = "x".repeat(1000);
        assert!(summarize("Bash", &json!({"command": long})).chars().count() <= 200);
    }

    #[test]
    fn results_per_tool() {
        assert_eq!(
            done("Read", json!({}), json!("1\ta\n2\tb\n3\tc")).as_deref(),
            Some("3 lines")
        );
        assert_eq!(
            done("Read", json!({}), json!("1\ta")).as_deref(),
            Some("1 line")
        );
        let edit = json!({"old_string": "a\nb\n", "new_string": "a\nc\nd\n"});
        assert_eq!(
            done("Edit", edit, json!("updated")).as_deref(),
            Some("+2 \u{2212}1")
        );
        assert_eq!(done("Bash", json!({"command": "ls"}), json!("out")), None);
        assert_eq!(
            done("Grep", json!({}), json!("a.rs\nb.rs\n")).as_deref(),
            Some("2 files")
        );
        assert_eq!(
            done("Glob", json!({}), json!([{"type": "text", "text": "a.rs"}])).as_deref(),
            Some("1 file")
        );
        assert_eq!(
            done("Glob", json!({}), json!("No files found")).as_deref(),
            Some("0 files")
        );
    }

    #[test]
    fn a_failure_keeps_its_first_line_and_its_output() {
        let (status, line, out) = result_of("Bash", &json!({}), &json!("Exit code 1\nboom"), true);
        assert_eq!(status, CallStatus::Failed);
        assert_eq!(line.as_deref(), Some("Exit code 1"));
        assert_eq!(out.as_deref(), Some("Exit code 1\nboom"));
    }

    #[test]
    fn failed_output_is_capped() {
        let big = (0..10_000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (_, _, out) = result_of("Bash", &json!({}), &json!(big), true);
        let out = out.unwrap();
        assert!(out.lines().count() <= OUTPUT_LINES);
        assert!(out.len() <= OUTPUT_BYTES);

        let wide = "é".repeat(OUTPUT_BYTES);
        let (_, _, out) = result_of("Bash", &json!({}), &json!(wide), true);
        assert!(out.unwrap().len() <= OUTPUT_BYTES);
    }
}
