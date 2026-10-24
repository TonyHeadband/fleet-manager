//! Incremental parser for `text/event-stream`. Bytes arrive in arbitrary chunks, so it works on bytes and
//! decodes a line only once the line is complete; a character split across chunks is never broken.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseMessage {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    id: Option<String>,
    event: Option<String>,
    data: Vec<String>,
    saw_data: bool,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// Feeds one chunk and returns every message it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseMessage> {
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
        out
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
        self.saw_data = false;
        message
    }
}
