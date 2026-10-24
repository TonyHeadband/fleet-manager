use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }

    fn from_str(s: &str) -> Option<Level> {
        Some(match s {
            "ERROR" => Level::Error,
            "WARN" => Level::Warn,
            "INFO" => Level::Info,
            "DEBUG" => Level::Debug,
            "TRACE" => Level::Trace,
            _ => return None,
        })
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Toast severity carried by the `notify` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Error,
    Warning,
    Success,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Unix seconds, UTC.
    pub time: i64,
    /// `None` for a line that is not in the log format (a panic, stderr noise).
    pub level: Option<Level>,
    pub target: String,
    /// The whole raw line when the line is not in the log format.
    pub message: String,
    pub fields: Vec<(String, String)>,
}

impl Entry {
    pub fn notice(&self) -> Option<Notice> {
        let (_, v) = self.fields.iter().find(|(k, _)| k == "notify")?;
        match v.as_str() {
            "error" => Some(Notice::Error),
            "warning" => Some(Notice::Warning),
            "success" => Some(Notice::Success),
            "info" => Some(Notice::Info),
            _ => None,
        }
    }

    pub fn parsed(&self) -> bool {
        self.level.is_some()
    }
}

// Hinnant's days_from_civil / civil_from_days; hand-rolled so the crate needs no
// date dependency for one fixed-width UTC format.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn format_time(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let shape_ok = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z';
    if !shape_ok {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(y, m as u32, d as u32) * 86_400 + h * 3_600 + mi * 60 + sec)
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            // Lenient on foreign input: keep an unknown escape verbatim.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn render(e: &Entry) -> String {
    let level = e.level.map_or("", Level::as_str);
    let mut out = format!("{} {:<5} {} ", format_time(e.time), level, e.target);
    escape(&e.message, &mut out);
    for (k, v) in &e.fields {
        out.push('\t');
        out.push_str(k);
        out.push('=');
        escape(v, &mut out);
    }
    out
}

pub fn parse(line: &str) -> Option<Entry> {
    let mut parts = line.split('\t');
    let head = parts.next()?;

    let (time, rest) = head.split_once(' ')?;
    let time = parse_time(time)?;
    let (level, rest) = rest.split_once(' ')?;
    let level = Level::from_str(level)?;
    // Padding after a short level name ("INFO ") leaves a leading space.
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let (target, message) = rest.split_once(' ').unwrap_or((rest, ""));
    if target.is_empty() {
        return None;
    }

    let mut fields = Vec::new();
    for part in parts {
        let (k, v) = part.split_once('=')?;
        fields.push((k.to_string(), unescape(v)));
    }

    Some(Entry {
        time,
        level: Some(level),
        target: target.to_string(),
        message: unescape(message),
        fields,
    })
}

pub fn parse_file(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut last_time = 0;
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let entry = parse(line).unwrap_or_else(|| Entry {
            time: last_time,
            level: None,
            target: String::new(),
            message: line.to_string(),
            fields: Vec::new(),
        });
        last_time = entry.time;
        entries.push(entry);
    }
    entries
}
