use fleet_client::attach::{split_input, ws_url};

#[test]
fn ctrl_close_bracket_detaches_and_nothing_after_it_is_sent() {
    assert_eq!(split_input(b"ab\x1dcd"), (b"ab".to_vec(), true));
    assert_eq!(split_input(b"abc"), (b"abc".to_vec(), false));
    assert_eq!(split_input(b"\x1d"), (Vec::new(), true));
}

#[test]
fn the_websocket_url_comes_from_the_daemon_url() {
    assert_eq!(
        ws_url("http://127.0.0.1:7878", "a1").unwrap(),
        "ws://127.0.0.1:7878/api/agents/a1/attach"
    );
    assert_eq!(
        ws_url("http://127.0.0.1:7878/", "a1").unwrap(),
        "ws://127.0.0.1:7878/api/agents/a1/attach"
    );
    assert_eq!(
        ws_url("http://[::1]:7878", "a1").unwrap(),
        "ws://[::1]:7878/api/agents/a1/attach"
    );
    assert!(ws_url("ftp://x", "a1").is_err());
    assert!(ws_url("http://127.0.0.1:7878", "../x").is_err());
}

#[test]
fn the_exit_message_carries_the_code() {
    use fleet_client::attach::exit_code;
    assert_eq!(exit_code(r#"{"exited":3}"#), Some(3));
    assert_eq!(exit_code(r#"{"exited":0}"#), Some(0));
    assert_eq!(exit_code(r#"{"other":1}"#), None);
    assert_eq!(exit_code("not json"), None);
}

#[test]
fn the_reset_undoes_what_an_agent_may_have_turned_on() {
    use fleet_client::attach::RESET;
    let nz = |n| std::num::NonZeroU16::new(n).unwrap();
    let mut term = vt100::Parser::new(nz(24), nz(80), 0);
    term.process(b"\x1b[?25l\x1b[?2004h\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?1049h");
    assert!(term.screen().alternate_screen() && term.screen().hide_cursor());
    term.process(RESET);
    let s = term.screen();
    assert!(!s.alternate_screen(), "still on the alternate screen");
    assert!(!s.hide_cursor(), "the cursor is still hidden");
    assert!(!s.bracketed_paste(), "bracketed paste is still on");
    assert_eq!(s.mouse_protocol_mode(), vt100::MouseProtocolMode::None);
    for mode in ["\x1b[?1004l", "\x1b[<u"] {
        assert!(
            RESET.windows(mode.len()).any(|w| w == mode.as_bytes()),
            "{mode:?}"
        );
    }
}

#[test]
fn attached_console_modes_add_vt_input_and_output_and_keep_the_rest() {
    use fleet_client::attach::{attached_modes, VT_INPUT, VT_PROCESSING};
    assert_eq!(
        attached_modes(0x0080, 0x0003),
        (0x0080 | VT_INPUT, 0x0003 | VT_PROCESSING)
    );
    assert_eq!(
        attached_modes(VT_INPUT, VT_PROCESSING),
        (VT_INPUT, VT_PROCESSING)
    );
}

#[test]
fn console_input_is_utf8_even_when_a_surrogate_pair_is_split() {
    use fleet_client::attach::Utf16ToUtf8;
    let mut d = Utf16ToUtf8::default();
    // ESC [ A (an arrow key as VT input), then U+1F600 split across two reads.
    assert_eq!(d.push(&[0x1b, b'[' as u16, b'A' as u16]), b"\x1b[A");
    assert_eq!(d.push(&[b'x' as u16, 0xD83D]), b"x");
    assert_eq!(d.push(&[0xDE00]), "\u{1F600}".as_bytes());
    // A lone Ctrl-Z is a key, not the end of input.
    assert_eq!(d.push(&[0x1a]), b"\x1a");
}
