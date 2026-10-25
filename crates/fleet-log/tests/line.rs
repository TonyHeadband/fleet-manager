use fleet_log::{format_time, parse, parse_file, render, Entry, Level, Notice};

fn entry(level: Level, message: &str, fields: &[(&str, &str)]) -> Entry {
    Entry {
        time: 1792852195,
        level: Some(level),
        target: "fleet_gui::app".into(),
        message: message.into(),
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn format_time_known_values() {
    assert_eq!(format_time(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_time(1792854595), "2026-10-24T15:09:55Z");
    assert_eq!(format_time(951782400), "2000-02-29T00:00:00Z");
}

#[test]
fn render_matches_the_spec_example() {
    let mut e = entry(
        Level::Error,
        "Could not start the agent: repo \"web\" not found",
        &[("notify", "error")],
    );
    e.time = 1792850995;
    assert_eq!(
        render(&e),
        "2026-10-24T14:09:55Z ERROR fleet_gui::app Could not start the agent: repo \"web\" not found\tnotify=error"
    );
}

#[test]
fn round_trip_with_escapes() {
    let e = entry(Level::Warn, "a\nb\tc\\d\re", &[("k", "x y\nz\\")]);
    let line = render(&e);
    assert!(!line.contains('\n') && !line.contains('\r'));
    assert_eq!(parse(&line), Some(e));
}

#[test]
fn level_is_padded_to_five() {
    let info = render(&entry(Level::Info, "m", &[]));
    let warn = render(&entry(Level::Warn, "m", &[]));
    assert!(info.contains("Z INFO  fleet_gui::app"), "{info}");
    assert!(warn.contains("Z WARN  fleet_gui::app"), "{warn}");
}

#[test]
fn garbage_is_not_parsed() {
    assert_eq!(parse("thread 'main' panicked at src/main.rs:3:5"), None);
}

#[test]
fn parse_file_attaches_unparsed_lines_to_the_previous_time() {
    let a = entry(Level::Info, "first", &[]);
    let mut b = entry(Level::Info, "second", &[]);
    b.time += 10;
    let panic = "thread 'main' panicked at src/main.rs:3:5";
    let text = format!("{}\n{}\n{}\n", render(&a), panic, render(&b));
    let entries = parse_file(&text);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], a);
    assert_eq!(entries[1].level, None);
    assert!(!entries[1].parsed());
    assert_eq!(entries[1].time, a.time);
    assert_eq!(entries[1].message, panic);
    assert_eq!(entries[1].target, "");
    assert_eq!(entries[2], b);
}

#[test]
fn parse_file_unparsed_first_line_has_time_zero() {
    let entries = parse_file("boom\n");
    assert_eq!(entries[0].time, 0);
}

#[test]
fn notice_reads_the_notify_field() {
    assert_eq!(
        entry(Level::Info, "m", &[("notify", "success")]).notice(),
        Some(Notice::Success)
    );
    assert_eq!(entry(Level::Info, "m", &[]).notice(), None);
    assert_eq!(
        entry(Level::Info, "m", &[("notify", "bogus")]).notice(),
        None
    );
}

fn time_of(stamp: &str) -> Option<i64> {
    parse(&format!("{stamp} INFO  t m")).map(|e| e.time)
}

#[test]
fn impossible_dates_and_times_are_not_parsed() {
    for bad in [
        "2026-02-29T00:00:00Z", // 2026 is not a leap year
        "2024-02-30T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-06-31T00:00:00Z",
        "2026-09-31T00:00:00Z",
        "2026-11-31T00:00:00Z",
        "1900-02-29T00:00:00Z", // divisible by 100, not by 400
        "2026-10-24T10:00:60Z",
        "2026-10-24T24:00:00Z",
        "2026-10-24T10:60:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-10-00T00:00:00Z",
        "2026-10-32T00:00:00Z",
        "2026-1a-01T00:00:00Z",
        "2026-10-24 10:00:00Z",
        "2026-10-24T10:00:00",
    ] {
        assert_eq!(time_of(bad), None, "{bad}");
    }
}

#[test]
fn the_last_day_of_each_month_parses() {
    for good in [
        "2024-02-29T00:00:00Z",
        "2000-02-29T00:00:00Z",
        "2026-01-31T23:59:59Z",
        "2026-04-30T00:00:00Z",
        "2026-12-31T00:00:00Z",
    ] {
        assert!(time_of(good).is_some(), "{good}");
    }
    assert_eq!(time_of("2000-02-29T00:00:00Z"), Some(951782400));
}
