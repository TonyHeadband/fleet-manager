//! Following a log file: its last `TAIL` bytes first, then what it gains, surviving truncation and a line still
//! being written. egui's Settings → Log and the Tauri app's both read through it.
use crate::{parse_file, Entry};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The most read from the end of a log file; older lines stay on disk.
pub const TAIL: u64 = 1024 * 1024;

/// The last `max` bytes of `path`, cut at a line start, and the file's length when it was read. A file that fits
/// comes back whole.
pub fn tail(path: &Path, max: u64) -> io::Result<(String, u64)> {
    let (bytes, end) = tail_bytes(path, max)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), end))
}

// Bytes, not text: offsets must stay exact when a line holds invalid UTF-8.
pub fn tail_bytes(path: &Path, max: u64) -> io::Result<(Vec<u8>, u64)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut buf = Vec::new();
    if len <= max {
        file.take(len).read_to_end(&mut buf)?;
        return Ok((buf, len));
    }
    // Starts one byte early: when that byte is a newline, the cut already is a line start.
    let from = len - max - 1;
    file.seek(SeekFrom::Start(from))?;
    file.take(max + 1).read_to_end(&mut buf)?;
    let end = from + buf.len() as u64;
    let skip = if buf.first() == Some(&b'\n') {
        1
    } else {
        buf.iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |i| i + 1)
    };
    buf.drain(..skip);
    Ok((buf, end))
}

/// Lines read from a file and the offset just after the last whole one. A line still being written is left for the
/// next read, or it would show up twice, as two fragments.
pub struct Chunk {
    pub entries: Vec<Entry>,
    pub end: u64,
    /// The last entry is an unterminated line, shown now and replaced when its newline arrives.
    pub partial: bool,
}

/// `show_partial` also lists an unterminated last line: on the first read it is often all a crash left behind.
pub fn lines(bytes: &[u8], end: u64, show_partial: bool) -> Chunk {
    let keep = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let partial = show_partial && keep < bytes.len();
    let shown = if partial { bytes.len() } else { keep };
    let entries = parse_file(&String::from_utf8_lossy(&bytes[..shown]));
    Chunk {
        entries,
        end: end - (bytes.len() - keep) as u64,
        partial,
    }
}

pub fn whole_lines(bytes: &[u8], end: u64) -> Chunk {
    lines(bytes, end, false)
}

pub struct Update {
    /// The file shrank (truncated or rotated): `chunk` replaces what was shown.
    pub reset: bool,
    pub chunk: Chunk,
}

pub fn read_new(path: &Path, offset: u64) -> io::Result<Update> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let reset = len < offset;
    let start = if reset { 0 } else { offset };
    if len - start > TAIL {
        let (bytes, end) = tail_bytes(path, TAIL)?;
        return Ok(Update {
            reset: true,
            chunk: whole_lines(&bytes, end),
        });
    }
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(start))?;
    file.take(len - start).read_to_end(&mut bytes)?;
    Ok(Update {
        reset,
        chunk: whole_lines(&bytes, start + bytes.len() as u64),
    })
}

/// The first read of a file: its last `TAIL` bytes, an unterminated last line shown (often all a crash left).
pub fn read_tail(path: &Path) -> io::Result<Chunk> {
    tail_bytes(path, TAIL).map(|(bytes, end)| lines(&bytes, end, true))
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileState {
    Pending,
    /// The window does not know where the state directory is.
    Unknown,
    Missing,
    Failed(String),
    Read,
}

pub struct FileLog {
    pub path: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub offset: u64,
    pub state: FileState,
    /// The last entry is an unterminated line that the next read starts over from.
    pub partial: bool,
}

impl FileLog {
    pub fn new() -> FileLog {
        FileLog {
            path: None,
            entries: Vec::new(),
            offset: 0,
            state: FileState::Pending,
            partial: false,
        }
    }

    pub fn load(&mut self, result: io::Result<Chunk>) {
        match result {
            Ok(chunk) => {
                self.entries = chunk.entries;
                self.offset = chunk.end;
                self.partial = chunk.partial;
                self.state = FileState::Read;
            }
            Err(e) => {
                self.fail(e);
            }
        }
    }

    /// Whether anything a row or notice shows changed.
    pub fn fail(&mut self, e: io::Error) -> bool {
        let state = if e.kind() == io::ErrorKind::NotFound {
            FileState::Missing
        } else {
            FileState::Failed(e.to_string())
        };
        let changed = !self.entries.is_empty() || self.state != state;
        self.entries.clear();
        self.offset = 0;
        self.partial = false;
        self.state = state;
        changed
    }

    pub fn notice(&self, what: &str) -> Option<String> {
        let path = self.path.as_ref().map(|p| p.display().to_string());
        match (&self.state, path) {
            (FileState::Missing, Some(p)) => Some(format!("No {what} log at {p}")),
            (FileState::Failed(e), Some(p)) => {
                Some(format!("Cannot read the {what} log at {p}: {e}"))
            }
            (FileState::Unknown, _) => {
                Some(format!("No {what} log: the state directory is unknown"))
            }
            _ => None,
        }
    }
}

impl Default for FileLog {
    fn default() -> FileLog {
        FileLog::new()
    }
}

impl FileLog {
    /// Whether anything a row or notice shows changed.
    pub fn apply(&mut self, update: Update) -> bool {
        let Update { reset, chunk } = update;
        let was_read = self.state == FileState::Read;
        self.state = FileState::Read;
        let mut entries = chunk.entries;
        if reset {
            self.offset = chunk.end;
            self.partial = chunk.partial;
            self.entries = entries;
            return true;
        }
        if entries.is_empty() {
            // Still nothing whole after the held-back line: it stays as it is.
            self.offset = chunk.end;
            return !was_read;
        }
        // The read starts at the unterminated line shown earlier, which comes back whole now.
        if std::mem::take(&mut self.partial) {
            self.entries.pop();
        }
        // A chunk that starts with stray lines has no earlier line of its own to take a time from.
        if let Some(prev) = self.entries.last().map(|e| e.time) {
            for e in entries.iter_mut().take_while(|e| !e.parsed()) {
                e.time = prev;
            }
        }
        self.offset = chunk.end;
        self.partial = chunk.partial;
        self.entries.append(&mut entries);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(entries: Vec<Entry>, end: u64) -> Update {
        Update {
            reset: false,
            chunk: Chunk {
                entries,
                end,
                partial: false,
            },
        }
    }

    #[test]
    fn a_poll_that_finds_nothing_changes_nothing() {
        let mut file = FileLog::new();
        file.load(Ok(Chunk {
            entries: parse_file("2026-10-24T10:00:01Z INFO  t m\n"),
            end: 31,
            partial: false,
        }));
        assert!(!file.apply(chunk(vec![], 31)), "nothing new");
        assert!(file.apply(chunk(parse_file("2026-10-24T10:00:02Z INFO  t n\n"), 62)));
        assert_eq!(file.entries.len(), 2);
        assert!(
            file.fail(io::Error::from(io::ErrorKind::NotFound)),
            "the lines vanish"
        );
        assert!(
            !file.fail(io::Error::from(io::ErrorKind::NotFound)),
            "still missing"
        );
    }
}
