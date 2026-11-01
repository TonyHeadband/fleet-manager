//! Follows a Claude transcript that is still being written.
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Reads the complete lines appended to a JSONL file since the last read.
pub struct Tail {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl Tail {
    pub fn new(path: PathBuf) -> Self {
        Tail {
            path,
            offset: 0,
            partial: Vec::new(),
        }
    }

    pub fn read_new(&mut self) -> std::io::Result<Vec<String>> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        file.seek(SeekFrom::Start(self.offset))?;
        let mut fresh = Vec::new();
        file.read_to_end(&mut fresh)?;
        self.offset += fresh.len() as u64;
        self.partial.extend_from_slice(&fresh);
        let Some(end) = self.partial.iter().rposition(|b| *b == b'\n') else {
            return Ok(Vec::new());
        };
        let rest = self.partial.split_off(end + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        Ok(String::from_utf8_lossy(&complete)
            .lines()
            .map(str::to_string)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn append(path: &std::path::Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn a_partial_last_line_waits_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut tail = Tail::new(path.clone());
        append(&path, "{\"a\":");
        assert!(tail.read_new().unwrap().is_empty());
        append(&path, "1}\n{\"b\"");
        assert_eq!(tail.read_new().unwrap(), ["{\"a\":1}"]);
        append(&path, ":2}\n");
        assert_eq!(tail.read_new().unwrap(), ["{\"b\":2}"]);
        assert!(tail.read_new().unwrap().is_empty());
    }

    #[test]
    fn a_missing_file_is_no_lines_until_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("later.jsonl");
        let mut tail = Tail::new(path.clone());
        assert!(tail.read_new().unwrap().is_empty());
        append(&path, "x\r\ny\n");
        assert_eq!(tail.read_new().unwrap(), ["x", "y"]);
    }
}
