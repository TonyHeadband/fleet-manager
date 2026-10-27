//! Scripted runs (the demo recorder) steer the window through a file of cues instead of synthetic input: one line
//! per cue, appended while the app runs. `FleetApp` polls it every frame.
use crate::components::{PreviewTab, ReleaseFilter};
use crate::views::{management::ManagementTab, View};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cue {
    /// Open the agent's side panel, as a click on its card does, on this tab.
    Open {
        agent: String,
        tab: PreviewTab,
    },
    Close,
    /// Switch to a view; `tab` picks a Management tab and is only given for that view.
    View {
        view: View,
        tab: Option<ManagementTab>,
    },
    Filter(ReleaseFilter),
    /// Make this persona the active one, as a Settings choice does.
    Persona(String),
    /// Close the window as its close button would.
    Quit,
}

fn view(word: &str) -> Option<View> {
    Some(match word {
        "kanban" => View::Kanban,
        "tasks" => View::Tasks,
        "releases" => View::Releases,
        "management" => View::Management,
        "settings" => View::Settings,
        _ => return None,
    })
}

fn management_tab(word: &str) -> Option<ManagementTab> {
    Some(match word {
        "overview" => ManagementTab::Overview,
        "products" => ManagementTab::Products,
        "personas" => ManagementTab::Personas,
        "instructions" => ManagementTab::Instructions,
        "agent_settings" => ManagementTab::AgentSettings,
        "orchestrators" => ManagementTab::Orchestrators,
        "specialists" => ManagementTab::Specialists,
        "skills" => ManagementTab::Skills,
        "workflow" => ManagementTab::Workflow,
        _ => return None,
    })
}

/// One cue per line (see the `Cue` variants for the words); anything else is logged and skipped.
pub fn parse_cue(line: &str) -> Option<Cue> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let cue = match words[..] {
        ["open", agent, tab] => match tab {
            "terminal" => Some(PreviewTab::Terminal),
            "diff" => Some(PreviewTab::Diff),
            "breakdown" => Some(PreviewTab::Breakdown),
            _ => None,
        }
        .map(|tab| Cue::Open {
            agent: agent.to_string(),
            tab,
        }),
        ["close"] => Some(Cue::Close),
        ["quit"] => Some(Cue::Quit),
        ["view", v] => view(v).map(|view| Cue::View { view, tab: None }),
        // A tab only means something on Management.
        ["view", "management", tab] => management_tab(tab).map(|tab| Cue::View {
            view: View::Management,
            tab: Some(tab),
        }),
        ["filter", "all"] => Some(Cue::Filter(ReleaseFilter::All)),
        ["filter", "backlog"] => Some(Cue::Filter(ReleaseFilter::Backlog)),
        ["filter", id] => id
            .parse()
            .ok()
            .map(|id| Cue::Filter(ReleaseFilter::Release(id))),
        ["persona", id] => Some(Cue::Persona(id.to_string())),
        _ => None,
    };
    if cue.is_none() {
        tracing::warn!("Ignoring GUI cue {line:?}");
    }
    cue
}

pub struct CueFile {
    path: PathBuf,
    /// Bytes already read, up to the end of the last complete line.
    offset: u64,
}

impl CueFile {
    pub fn new(path: PathBuf) -> CueFile {
        CueFile { path, offset: 0 }
    }

    /// Starts reading at `offset`, so a relaunched window does not replay the cues its predecessor applied.
    pub fn from_offset(path: PathBuf, offset: u64) -> CueFile {
        CueFile { path, offset }
    }

    /// The cues appended since the last poll. A line without its newline yet is left for a later poll, so a cue
    /// caught mid-write is never read in halves.
    pub fn poll(&mut self) -> Vec<Cue> {
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return vec![];
        };
        let mut buf = Vec::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_end(&mut buf).is_err() {
            return vec![];
        }
        let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
            return vec![];
        };
        self.offset += end as u64 + 1;
        String::from_utf8_lossy(&buf[..end])
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(parse_cue)
            .collect()
    }
}
