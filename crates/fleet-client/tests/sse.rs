use fleet_client::sse::{SseMessage, SseParser};

fn msg(id: Option<&str>, event: Option<&str>, data: &str) -> SseMessage {
    SseMessage { id: id.map(String::from), event: event.map(String::from), data: data.into() }
}

fn all(chunks: &[&[u8]]) -> Vec<SseMessage> {
    let mut p = SseParser::new();
    chunks.iter().flat_map(|c| p.feed(c)).collect()
}

#[test]
fn parses_id_event_and_data() {
    let got = all(&[b"id: 7\nevent: event\ndata: {\"seq\":7}\n\n"]);
    assert_eq!(got, [msg(Some("7"), Some("event"), "{\"seq\":7}")]);
}

#[test]
fn frame_split_across_chunks_at_every_byte_offset_gives_the_same_messages() {
    // Includes a two-byte character so some splits fall inside it.
    let stream = "event: snapshot\nid: 3\ndata: {\"task\":\"café\"}\n\nid: 4\nevent: event\ndata: x\n\n".as_bytes();
    let whole = all(&[stream]);
    assert_eq!(whole.len(), 2);
    for k in 0..=stream.len() {
        assert_eq!(all(&[&stream[..k], &stream[k..]]), whole, "split at {k}");
    }
    assert_eq!(whole[0].data, "{\"task\":\"café\"}");
}

#[test]
fn accepts_crlf_and_lf_endings() {
    let a = all(&[b"id: 1\r\nevent: e\r\ndata: x\r\n\r\n"]);
    let b = all(&[b"id: 1\nevent: e\ndata: x\n\n"]);
    assert_eq!(a, b);
    assert_eq!(a, [msg(Some("1"), Some("e"), "x")]);
}

#[test]
fn skips_comment_lines_and_keepalives() {
    let got = all(&[b": keep-alive\n\n: another\nid: 2\ndata: y\n\n"]);
    assert_eq!(got, [msg(Some("2"), None, "y")]);
}

#[test]
fn joins_multiple_data_lines_with_a_newline() {
    let got = all(&[b"data: line one\ndata: line two\n\n"]);
    assert_eq!(got, [msg(None, None, "line one\nline two")]);
}

#[test]
fn a_named_event_with_empty_data_is_still_delivered() {
    // The daemon ends a terminal stream with `event: closed` and no payload.
    let got = all(&[b"event: closed\ndata:\n\n"]);
    assert_eq!(got, [msg(None, Some("closed"), "")]);
}

#[test]
fn ignores_a_frame_with_no_fields() {
    assert!(all(&[b"\n\n\nretry: 1000\n\n"]).is_empty());
}

#[test]
fn a_trailing_partial_frame_waits_for_more_bytes() {
    let mut p = SseParser::new();
    assert!(p.feed(b"id: 9\nevent: event\ndata: par").is_empty());
    assert!(p.feed(b"tial").is_empty());
    assert_eq!(p.feed(b"\n\n"), [msg(Some("9"), Some("event"), "partial")]);
}

#[test]
fn a_value_keeps_everything_after_the_first_colon_and_one_space() {
    let got = all(&[b"data: a: b  c\n\n"]);
    assert_eq!(got, [msg(None, None, "a: b  c")]);
}
