//! Incremental parser for `text/event-stream`. Bytes arrive in arbitrary chunks, so it works on bytes and
//! decodes a line only once the line is complete; a character split across chunks is never broken.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseMessage {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
}

/// Most bytes one message may take while it is read: a snapshot of many agents is the largest the daemon sends. A
/// server that goes past it without ending a line or a message is broken or not the daemon, and would otherwise
/// grow the buffer until the app runs out of memory.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// A line or message went past the parser's limit; its state is dropped and the stream should be closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

pub struct SseParser {
    buf: Vec<u8>,
    id: Option<String>,
    event: Option<String>,
    data: Vec<String>,
    /// Bytes in `data`, so the limit check does not sum it per chunk.
    data_bytes: usize,
    saw_data: bool,
    limit: usize,
}

impl Default for SseParser {
    fn default() -> SseParser {
        SseParser::with_limit(MAX_MESSAGE_BYTES)
    }
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    pub fn with_limit(limit: usize) -> SseParser {
        SseParser {
            buf: Vec::new(),
            id: None,
            event: None,
            data: Vec::new(),
            data_bytes: 0,
            saw_data: false,
            limit,
        }
    }

    /// Feeds one chunk and returns every message it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseMessage>, TooLarge> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=end).collect();
            line.pop(); // the \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(m) = self.line(&String::from_utf8_lossy(&line)) {
                out.push(m);
            }
        }
        // Checked after the complete lines are taken: only what is still unfinished counts.
        if self.buf.len() + self.data_bytes > self.limit {
            *self = SseParser::with_limit(self.limit);
            return Err(TooLarge);
        }
        Ok(out)
    }

    fn line(&mut self, line: &str) -> Option<SseMessage> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "id" => self.id = Some(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.saw_data = true;
                self.data_bytes += value.len();
                self.data.push(value.to_string());
            }
            _ => {} // retry and unknown fields are ignored
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseMessage> {
        let message = (self.saw_data || self.event.is_some()).then(|| SseMessage {
            id: self.id.clone(),
            event: self.event.clone(),
            data: self.data.join("\n"),
        });
        self.event = None;
        self.data.clear();
        self.data_bytes = 0;
        self.saw_data = false;
        message
    }
}
