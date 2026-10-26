use fleet_demo::hooks::Sink;
use fleet_demo::input::Input;
use fleet_demo::player::Player;
use fleet_demo::scenario::{FollowUp, Scenario, Step, StepKind};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

fn step(kind: StepKind) -> Step {
    Step {
        after: Duration::from_secs(2),
        kind,
    }
}

fn tool(name: &str, input: &str) -> StepKind {
    StepKind::Tool {
        tool: name.into(),
        input: input.into(),
        output: vec!["one".into(), "two".into()],
    }
}

fn scenario(main: Vec<StepKind>, follow_ups: Vec<FollowUp>) -> Scenario {
    Scenario {
        task: "bake".into(),
        main: main.into_iter().map(step).collect(),
        follow_ups,
    }
}

struct Run {
    events: Vec<Value>,
    transcript: String,
}

impl Run {
    fn names(&self) -> Vec<&str> {
        self.events
            .iter()
            .map(|e| e["hook_event_name"].as_str().unwrap())
            .collect()
    }
}

fn play(s: Scenario, inputs: Vec<Input>, cwd: &Path, dir: &Path) -> Run {
    let (tx, rx) = mpsc::channel();
    for i in inputs {
        tx.send(i).unwrap();
    }
    tx.send(Input::Eof).unwrap();
    let mut out: Vec<u8> = Vec::new();
    let events = {
        let mut p = Player::new(
            s,
            Vec::<Value>::new(),
            &mut out,
            cwd.to_path_buf(),
            "sess".into(),
            dir.to_path_buf(),
            |_| {},
        );
        p.run(&rx).unwrap();
        p.sink().clone()
    };
    Run {
        events,
        transcript: String::from_utf8(out).unwrap(),
    }
}

fn line(s: &str) -> Input {
    Input::Line(s.into())
}

fn quick(s: Scenario, inputs: Vec<Input>) -> Run {
    let d = tempfile::tempdir().unwrap();
    play(s, inputs, d.path(), d.path())
}

#[test]
fn plays_main_arc_events_in_order() {
    let s = scenario(
        vec![
            StepKind::Say("Looking".into()),
            tool("Bash", "ls -la"),
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(s, vec![]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(r.events[1]["prompt"], "bake");
    assert_eq!(r.events[2]["tool_input"]["command"], "ls -la");
    assert!(r.transcript.contains("• Looking"));
    assert!(r.transcript.contains("• Bash(ls -la)"));
    assert!(r.transcript.contains("└  one"));
}

#[test]
fn permission_waits_for_a_line_then_continues() {
    let s = scenario(
        vec![
            StepKind::Permission {
                tool: "Bash".into(),
                input: "rm -rf dough".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(s, vec![line("yes")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "PermissionRequest",
            "PostToolUse",
            "Stop",
            "SessionEnd"
        ]
    );
    assert!(r.transcript.contains("rm -rf dough"));
    assert!(r.transcript.contains("> yes"));
}

#[test]
fn question_posts_elicitation_then_user_prompt() {
    let s = scenario(
        vec![StepKind::Question {
            text: "Which flour?".into(),
        }],
        vec![],
    );
    let r = quick(s, vec![line("rye")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "UserPromptSubmit",
            "SessionEnd"
        ]
    );
    assert_eq!(r.events[2]["notification_type"], "elicitation_dialog");
    assert_eq!(r.events[3]["prompt"], "rye");
    assert!(r.transcript.contains("Which flour?"));
}

#[test]
fn early_answer_is_kept_for_the_next_wait() {
    let s = scenario(
        vec![
            StepKind::Say("warming up".into()),
            StepKind::Question {
                text: "Sure?".into(),
            },
        ],
        vec![],
    );
    let r = quick(s, vec![line("early")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "UserPromptSubmit",
            "SessionEnd"
        ]
    );
    assert_eq!(r.events[3]["prompt"], "early");
    assert!(r.transcript.contains("> early"));
}

#[test]
fn escape_during_a_wait_then_continue_stays_in_the_wait() {
    let s = scenario(
        vec![
            StepKind::Question {
                text: "Sure?".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(s, vec![Input::Escape, line("continue"), line("yes")]);
    assert!(r.transcript.contains("Interrupted"));
    assert_eq!(r.events[3]["prompt"], "yes");
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "UserPromptSubmit",
            "Stop",
            "SessionEnd"
        ]
    );
}

#[test]
fn escape_then_other_line_abandons_the_arc_and_handles_it_as_a_prompt() {
    let s = scenario(
        vec![
            StepKind::Question {
                text: "Sure?".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(s, vec![Input::Escape, line("stop that")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "UserPromptSubmit",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(r.events[2]["prompt"], "stop that");
}

#[test]
fn follow_up_line_plays_its_arc_and_stops() {
    let s = scenario(
        vec![StepKind::Stop],
        vec![FollowUp {
            on: "add jam".into(),
            steps: vec![
                step(StepKind::Say("Jam added".into())),
                step(StepKind::Stop),
            ],
        }],
    );
    let r = quick(s, vec![line("add jam"), line("whatever")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Stop",
            "UserPromptSubmit",
            "Stop",
            "UserPromptSubmit",
            "Stop",
            "SessionEnd"
        ]
    );
    assert!(r.transcript.contains("• Jam added"));
    assert_eq!(r.events[5]["prompt"], "whatever");
}

#[test]
fn write_step_commits_in_the_repo_worktree() {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path().join("task");
    let repo = cwd.join("bakery");
    std::fs::create_dir_all(&repo).unwrap();
    let scen = d.path().join("scen");
    std::fs::create_dir_all(&scen).unwrap();
    std::fs::write(scen.join("loaf.txt"), "crust").unwrap();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "T"]);
    let s = scenario(
        vec![StepKind::Write {
            path: "src/loaf.txt".into(),
            from: "loaf.txt".into(),
            commit: Some("feat: add loaf".into()),
        }],
        vec![],
    );
    let r = play(s, vec![], &cwd, &scen);
    assert_eq!(
        std::fs::read_to_string(repo.join("src/loaf.txt")).unwrap(),
        "crust"
    );
    let log = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["log", "-1", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "feat: add loaf"
    );
    assert_eq!(r.events[2]["tool_name"], "Write");
    assert!(r.transcript.contains("• Write(src/loaf.txt)"));
}

#[test]
fn eof_posts_session_end() {
    let r = quick(scenario(vec![], vec![]), vec![]);
    assert_eq!(
        r.names(),
        ["SessionStart", "UserPromptSubmit", "SessionEnd"]
    );
}

#[test]
fn escape_keeps_an_earlier_queued_answer_for_the_next_wait() {
    let s = scenario(
        vec![
            StepKind::Question {
                text: "Sure?".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(s, vec![line("yes"), Input::Escape, line("continue")]);
    assert_eq!(
        r.names(),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "UserPromptSubmit",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(r.events[3]["prompt"], "yes");
}

#[derive(Clone)]
struct Shared(Arc<Mutex<Vec<Value>>>);

impl Sink for Shared {
    fn send(&mut self, body: Value) {
        self.0.lock().unwrap().push(body);
    }
}

#[test]
fn escape_while_needs_you_then_continue_keeps_the_question_open() {
    let s = scenario(
        vec![
            StepKind::Question {
                text: "Sure?".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let watcher = {
        let events = events.clone();
        std::thread::spawn(move || {
            // Inputs arrive only once the question is posted, so the player is inside the wait.
            let started = std::time::Instant::now();
            while !events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e["hook_event_name"] == "Notification")
            {
                assert!(started.elapsed() < Duration::from_secs(5), "no question");
                std::thread::sleep(Duration::from_millis(5));
            }
            for i in [Input::Escape, line("continue"), line("yes"), Input::Eof] {
                tx.send(i).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };
    let d = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut p = Player::new(
        s,
        Shared(events.clone()),
        &mut out,
        d.path().to_path_buf(),
        "sess".into(),
        d.path().to_path_buf(),
        |_| {},
    );
    p.run(&rx).unwrap();
    watcher.join().unwrap();
    let ev = events.lock().unwrap();
    let names: Vec<&str> = ev
        .iter()
        .map(|e| e["hook_event_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "SessionStart",
            "UserPromptSubmit",
            "Notification",
            "UserPromptSubmit",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(ev[3]["prompt"], "yes");
    assert!(String::from_utf8(out).unwrap().contains("Interrupted"));
}

/// The GUI's Terminal tab draws the pane in IBM Plex Mono; a glyph it lacks shows as an empty box in the demo.
#[test]
fn transcript_uses_only_glyphs_the_gui_font_has() {
    use skrifa::{FontRef, MetadataProvider};
    let s = scenario(
        vec![
            StepKind::Say("Looking".into()),
            tool("Bash", "ls -la"),
            StepKind::Permission {
                tool: "Bash".into(),
                input: "oven bake".into(),
            },
            StepKind::Question {
                text: "Sure?".into(),
            },
            StepKind::Stop,
        ],
        vec![],
    );
    let r = quick(
        s,
        vec![line("1"), Input::Escape, line("continue"), line("yes")],
    );
    assert!(r.transcript.contains("Interrupted"), "{}", r.transcript);
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fleet-gui/assets/fonts/IBMPlexMono-Regular.ttf"),
    )
    .unwrap();
    let charmap = FontRef::new(&bytes).unwrap().charmap();
    let missing: Vec<char> = r
        .transcript
        .chars()
        .filter(|c| !c.is_control() && charmap.map(*c).is_none())
        .collect();
    assert!(missing.is_empty(), "not in IBM Plex Mono: {missing:?}");
}
