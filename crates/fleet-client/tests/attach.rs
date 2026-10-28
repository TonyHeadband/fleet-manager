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
