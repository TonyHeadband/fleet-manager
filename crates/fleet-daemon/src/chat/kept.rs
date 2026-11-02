//! The chats an archive keeps. An archive deletes the agents, their sessions and their folders, and Claude Code (or
//! deleting a persona) takes the transcripts, so before that each agent's chat is copied to
//! `<state_dir>/transcripts/<task-id>/<agent-id>/`: its PROMPT.md, each session's `.jsonl` and a `chat.json` saying
//! what the store would have. Archive never deletes that folder.
use crate::agents::PROMPT_FILE;
use fleet_api::AgentKind;
use serde::{Deserialize, Serialize};
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

/// The folder of kept chats, in the state dir.
pub const DIR: &str = "transcripts";
/// What is known about the chat, beside its files.
pub const CHAT_FILE: &str = "chat.json";

/// What the store knew of an agent's chat when its task was archived.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptChat {
    pub kind: AgentKind,
    /// The Claude sessions, oldest first, with the time each began.
    pub sessions: Vec<(String, i64)>,
    /// When the agent stopped: the chat ends there.
    pub ended: i64,
}

/// An agent's chat as it is about to be copied.
#[derive(Clone, Debug)]
pub struct Source {
    pub agent: String,
    pub chat: KeptChat,
    /// The agent's PROMPT.md, if it has one.
    pub prompt: Option<PathBuf>,
    /// Each session that has a transcript file: its id and where the file is.
    pub transcripts: Vec<(String, PathBuf)>,
}

/// A name that is one path component and no more: an id from a URL or a file is joined to a path only if it is.
pub fn is_plain(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(std::path::Component::Normal(n)), None) if n == name
    ) && !name.contains(['/', '\\', '\0'])
}

fn chat_json(chat: &KeptChat) -> std::io::Result<Vec<u8>> {
    serde_json::to_vec(chat).map_err(Error::other)
}

fn len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// What copying `sources` writes. Blocking.
pub fn bytes(sources: &[Source]) -> u64 {
    sources
        .iter()
        .map(|s| {
            let files = s.prompt.iter().chain(s.transcripts.iter().map(|(_, p)| p));
            files.map(|p| len(p)).sum::<u64>() + chat_json(&s.chat).map_or(0, |j| j.len() as u64)
        })
        .sum()
}

/// Copies `sources` to `<root>/<task>/<agent>/`: into a temp folder first, then renamed into place, so the folder is
/// there whole or not at all; one left by an earlier try is replaced. A file the source no longer has (an earlier
/// try already deleted the PROMPT.md, or a transcript) but the earlier copy does is carried over, so a retry never
/// loses what an earlier try kept. On any error the temp folder is removed and nothing else of `root` is changed.
/// A kept folder or temp folder that is a symbolic link is refused, not replaced. Blocking.
pub fn keep(root: &Path, task: &str, sources: &[Source]) -> std::io::Result<()> {
    let plain = |n: &str| {
        is_plain(n).then_some(()).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("{n:?} is not a plain name"),
            )
        })
    };
    plain(task)?;
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!(".{task}.tmp"));
    let earlier = root.join(task);
    for path in [&tmp, &earlier] {
        refuse_link(path)?;
    }
    crate::archive::remove_whole(&tmp)?;
    let copied = (|| {
        for s in sources {
            plain(&s.agent)?;
            let dir = tmp.join(&s.agent);
            std::fs::create_dir_all(&dir)?;
            let before = earlier.join(&s.agent);
            match &s.prompt {
                Some(prompt) => {
                    std::fs::copy(prompt, dir.join(PROMPT_FILE))?;
                }
                None => carry_over(&before, &dir, PROMPT_FILE)?,
            }
            for (session, path) in &s.transcripts {
                plain(session)?;
                std::fs::copy(path, dir.join(format!("{session}.jsonl")))?;
            }
            for (session, _) in &s.chat.sessions {
                let name = format!("{session}.jsonl");
                if is_plain(session) && !dir.join(&name).exists() {
                    carry_over(&before, &dir, &name)?;
                }
            }
            std::fs::write(dir.join(CHAT_FILE), chat_json(&s.chat)?)?;
        }
        // A leftover of an earlier try goes only now, so a failed copy changes nothing.
        crate::archive::remove_whole(&earlier)?;
        std::fs::rename(&tmp, &earlier)
    })();
    if copied.is_err() {
        let _ = crate::archive::remove_whole(&tmp);
    }
    copied
}

/// Fails when `path` is a symbolic link: nothing is replaced or deleted through one.
fn refuse_link(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{} is a symbolic link", path.display()),
        )),
        _ => Ok(()),
    }
}

/// Copies `name` from the agent's folder of an earlier try into `dir`, when that is a regular file there.
fn carry_over(before: &Path, dir: &Path, name: &str) -> std::io::Result<()> {
    let is_dir = std::fs::symlink_metadata(before).is_ok_and(|m| m.is_dir());
    let old = before.join(name);
    if is_dir && std::fs::symlink_metadata(&old).is_ok_and(|m| m.is_file()) {
        std::fs::copy(&old, dir.join(name))?;
    }
    Ok(())
}

/// A kept chat: where its files are, and what `chat.json` says.
#[derive(Clone, Debug)]
pub struct Kept {
    pub dir: PathBuf,
    pub chat: KeptChat,
}

impl Kept {
    /// The transcript of `session`, when kept.
    pub fn transcript(&self, session: &str) -> Option<PathBuf> {
        let path = self.dir.join(format!("{session}.jsonl"));
        (is_plain(session) && path.is_file()).then_some(path)
    }

    pub fn prompt(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join(PROMPT_FILE)).ok()
    }
}

/// The kept chat of `agent` in whichever task's folder holds it. None when the id is not a plain name or no
/// folder has it. Blocking.
pub fn find(state_dir: &Path, agent: &str) -> Option<Kept> {
    if !is_plain(agent) {
        return None;
    }
    let root = state_dir.join(DIR);
    let mut tasks: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    tasks.sort();
    tasks.into_iter().find_map(|task| {
        let dir = task.join(agent);
        let file = dir.join(CHAT_FILE);
        let real = std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir())
            && std::fs::symlink_metadata(&file).is_ok_and(|m| m.is_file());
        let chat = serde_json::from_slice(&std::fs::read(&file).ok().filter(|_| real)?).ok()?;
        Some(Kept { dir, chat })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(agent: &str, prompt: Option<PathBuf>) -> Source {
        Source {
            agent: agent.into(),
            chat: KeptChat {
                kind: AgentKind::Solo,
                sessions: vec![],
                ended: 5,
            },
            prompt,
            transcripts: vec![],
        }
    }

    #[test]
    fn a_failed_copy_leaves_nothing_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("transcripts");
        std::fs::create_dir_all(root.join("t")).unwrap();
        std::fs::write(root.join("t/earlier"), "x").unwrap();
        let ok = tmp.path().join("PROMPT.md");
        std::fs::write(&ok, "hi").unwrap();
        // The second agent's PROMPT.md is gone by the time it is copied.
        let sources = [
            source("a", Some(ok)),
            source("b", Some(tmp.path().join("gone"))),
        ];

        assert!(keep(&root, "t", &sources).is_err());

        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["t"], "no temp folder is left");
        assert_eq!(
            std::fs::read_to_string(root.join("t/earlier")).unwrap(),
            "x"
        );
    }

    #[test]
    fn a_file_the_source_lost_is_carried_over_from_the_earlier_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("transcripts");
        let old = root.join("t/a");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join(PROMPT_FILE), "the prompt").unwrap();
        std::fs::write(old.join("s1.jsonl"), "one").unwrap();
        std::fs::write(old.join("s2.jsonl"), "old two").unwrap();
        let fresh = tmp.path().join("s2.jsonl");
        std::fs::write(&fresh, "new two").unwrap();
        let mut src = source("a", None);
        src.chat.sessions = vec![("s1".into(), 1), ("s2".into(), 2)];
        src.transcripts = vec![("s2".into(), fresh)];

        keep(&root, "t", &[src]).unwrap();

        let read = |n: &str| std::fs::read_to_string(root.join("t/a").join(n)).unwrap();
        assert_eq!(read(PROMPT_FILE), "the prompt");
        assert_eq!(read("s1.jsonl"), "one");
        assert_eq!(read("s2.jsonl"), "new two", "a source that exists wins");
    }

    #[test]
    fn a_task_or_agent_that_is_not_a_plain_name_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(keep(tmp.path(), "../x", &[]).is_err());
        assert!(keep(tmp.path(), "t", &[source("../x", None)]).is_err());
        assert!(!tmp.path().join("t").exists());
    }

    #[test]
    fn only_a_single_plain_component_is_a_name() {
        for ok in ["a", "0f3c9a", "agent-1", "a.b", ".hidden"] {
            assert!(is_plain(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", "a/b", "../a", "a/..", "/a", "a\\b", "a\0b", "C:\\x",
        ] {
            assert!(!is_plain(bad), "{bad:?}");
        }
    }
}
