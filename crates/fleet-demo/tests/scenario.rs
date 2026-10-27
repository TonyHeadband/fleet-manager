use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fleet_demo::scenario::{find, load_dir, parse, Scenario, StepKind};

const FULL: &str = r#"
task = "Make an all-butter crust"

[[step]]
say = "Starting on the dough."

[[step]]
after = "1.0s"
tool = "Bash"
input = "weigh flour --grams 300"
output = ["300 g all-purpose flour"]

[[step]]
after = "2s"
permission = { tool = "Bash", input = "fridge rest --minutes 30" }

[[step]]
question = "Salted or unsalted butter?"

[[step]]
write = { path = "recipe/crust.md", from = "content/crust.txt" }
commit = "Add the all-butter crust"

[[step]]
stop = true

[[follow_up]]
on = "now make it a lattice top"
[[follow_up.step]]
after = "0.8s"
say = "Cutting the top crust into strips."
"#;

fn parse_ok(text: &str) -> Scenario {
    parse(text, Path::new("crust.toml")).unwrap()
}

#[test]
fn parses_every_step_kind() {
    let s = parse_ok(FULL);
    assert_eq!(s.task, "Make an all-butter crust");
    let kinds: Vec<_> = s.main.iter().map(|st| st.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            StepKind::Say("Starting on the dough.".into()),
            StepKind::Tool {
                tool: "Bash".into(),
                input: "weigh flour --grams 300".into(),
                output: vec!["300 g all-purpose flour".into()],
            },
            StepKind::Permission {
                tool: "Bash".into(),
                input: "fridge rest --minutes 30".into(),
            },
            StepKind::Question {
                text: "Salted or unsalted butter?".into()
            },
            StepKind::Write {
                path: "recipe/crust.md".into(),
                from: "content/crust.txt".into(),
                commit: Some("Add the all-butter crust".into()),
            },
            StepKind::Stop,
        ]
    );
    assert_eq!(s.main[1].after, Duration::from_millis(1000));
    assert_eq!(s.main[2].after, Duration::from_secs(2));
    assert_eq!(s.follow_ups.len(), 1);
    assert_eq!(s.follow_ups[0].on, "now make it a lattice top");
    assert_eq!(s.follow_ups[0].steps[0].after, Duration::from_millis(800));
}

#[test]
fn after_defaults_to_zero() {
    let s = parse_ok("task = \"t\"\n[[step]]\nsay = \"hi\"\n");
    assert_eq!(s.main[0].after, Duration::ZERO);
}

#[test]
fn milliseconds_parse() {
    let s = parse_ok("task = \"t\"\n[[step]]\nafter = \"250ms\"\nsay = \"hi\"\n");
    assert_eq!(s.main[0].after, Duration::from_millis(250));
}

#[test]
fn bad_duration_is_an_error() {
    let err = parse(
        "task = \"t\"\n[[step]]\nafter = \"soon\"\nsay = \"hi\"\n",
        Path::new("x.toml"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("x.toml"));
    assert!(err.to_string().contains("soon"));
}

#[test]
fn two_kinds_in_one_step_is_an_error() {
    let err = parse(
        "task = \"t\"\n[[step]]\nsay = \"hi\"\nstop = true\n",
        Path::new("two.toml"),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("two.toml"), "{msg}");
    assert!(msg.contains("exactly one of"), "{msg}");
}

fn invalid(text: &str) -> String {
    parse(text, Path::new("bad.toml")).unwrap_err().to_string()
}

#[test]
fn commit_without_a_write_is_an_error() {
    let msg = invalid("task = \"t\"\n[[step]]\nsay = \"hi\"\ncommit = \"m\"\n");
    assert!(msg.contains("bad.toml"), "{msg}");
    assert!(
        msg.contains("`commit` only applies to a `write` step"),
        "{msg}"
    );
}

#[test]
fn stop_false_is_an_error() {
    let msg = invalid("task = \"t\"\n[[step]]\nstop = false\n");
    assert!(msg.contains("`stop = false` is not a step"), "{msg}");
}

#[test]
fn tool_without_input_is_an_error() {
    let msg = invalid("task = \"t\"\n[[step]]\ntool = \"Bash\"\n");
    assert!(msg.contains("tool step needs `input`"), "{msg}");
}

#[test]
fn unknown_key_inside_write_is_an_error() {
    let msg =
        invalid("task = \"t\"\n[[step]]\nwrite = { path = \"a\", from = \"b\", mode = \"x\" }\n");
    assert!(msg.contains("bad.toml"), "{msg}");
    assert!(msg.contains("mode"), "{msg}");
}

#[test]
fn a_huge_delay_is_an_error_not_a_panic() {
    let msg = invalid("task = \"t\"\n[[step]]\nafter = \"1e30s\"\nsay = \"hi\"\n");
    assert!(msg.contains("bad.toml"), "{msg}");
    assert!(msg.contains("1e30s"), "{msg}");
}

#[test]
fn step_without_a_kind_is_an_error() {
    assert!(parse(
        "task = \"t\"\n[[step]]\nafter = \"1s\"\n",
        Path::new("n.toml")
    )
    .is_err());
}

#[test]
fn unknown_key_is_an_error_naming_file_and_line() {
    let err = parse(
        "task = \"t\"\n[[step]]\nsay = \"hi\"\nbogus = 1\n",
        Path::new("typo.toml"),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("typo.toml"), "{msg}");
    assert!(msg.contains("line 4"), "{msg}");
}

#[test]
fn find_matches_the_trimmed_prompt() {
    let all = vec![(PathBuf::from("crust.toml"), parse_ok(FULL))];
    assert!(find(&all, "Make an all-butter crust\n").is_some());
    assert!(find(&all, "Make a crust").is_none());
}

#[test]
fn load_dir_skips_timeline_toml() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("crust.toml"), FULL).unwrap();
    fs::write(dir.path().join("timeline.toml"), "not = [a scenario").unwrap();
    fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
    let all = load_dir(dir.path()).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, dir.path().join("crust.toml"));
}

#[test]
fn call_step_parses_its_args_table() {
    let s = parse_ok(
        "task = \"t\"\n[[step]]\ncall = { tool = \"spawn_specialist\", args = { step = 1, persona = \"baker\" } }\n",
    );
    assert_eq!(
        s.main[0].kind,
        StepKind::Call {
            tool: "spawn_specialist".into(),
            args: serde_json::json!({ "step": 1, "persona": "baker" }),
        }
    );
}

#[test]
fn call_without_args_gets_an_empty_object() {
    let s = parse_ok("task = \"t\"\n[[step]]\ncall = { tool = \"wait_for_fleet\" }\n");
    assert_eq!(
        s.main[0].kind,
        StepKind::Call {
            tool: "wait_for_fleet".into(),
            args: serde_json::json!({}),
        }
    );
}

#[test]
fn merge_step_parses() {
    let s = parse_ok("task = \"t\"\n[[step]]\nafter = \"1s\"\nmerge = 2\n");
    assert_eq!(s.main[0].kind, StepKind::Merge(2));
    assert_eq!(s.main[0].after, Duration::from_secs(1));
}

#[test]
fn call_and_say_in_one_step_is_refused() {
    let msg = invalid("task = \"t\"\n[[step]]\nsay = \"hi\"\ncall = { tool = \"open_pr\" }\n");
    assert!(msg.contains("bad.toml"), "{msg}");
    assert!(msg.contains("exactly one of"), "{msg}");
}

#[test]
fn find_matches_a_specialist_by_its_brief() {
    let all = vec![(
        PathBuf::from("laminate.toml"),
        parse_ok("task = \"Laminate the dough\"\n[[step]]\nstop = true\n"),
    )];
    let found = find(
        &all,
        "Laminate the dough\n\nThe task's goal: flat croissants",
    );
    assert_eq!(found.map(|s| s.task.as_str()), Some("Laminate the dough"));
    assert!(find(&all, "Laminate\n\nthe dough").is_none());
}
