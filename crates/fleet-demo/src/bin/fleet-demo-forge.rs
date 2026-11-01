//! Stands in for GitHub's REST API in the demo: `GET /user` answers that the token is baker's, anything else is not
//! found. The daemon reaches it through `FLEET_FORGE_API`. It listens on a free loopback port and writes its base URL
//! to the file `--addr-file` names, then serves until killed.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(addr_file) = args
        .iter()
        .position(|a| a == "--addr-file")
        .and_then(|i| args.get(i + 1))
    else {
        eprintln!("usage: fleet-demo-forge --addr-file <path>");
        return ExitCode::from(2);
    };
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fleet-demo-forge: cannot listen: {e}");
            return ExitCode::FAILURE;
        }
    };
    let url = format!("http://{}", listener.local_addr().expect("a bound address"));
    // Through a sibling, so a script polling for the file never reads half of it.
    let tmp = format!("{addr_file}.tmp");
    if let Err(e) =
        std::fs::write(&tmp, format!("{url}\n")).and_then(|()| std::fs::rename(&tmp, addr_file))
    {
        eprintln!("fleet-demo-forge: cannot write {addr_file}: {e}");
        return ExitCode::FAILURE;
    }
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || answer(stream));
    }
    ExitCode::SUCCESS
}

fn answer(stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // The headers; no route here takes a body.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }
    let mut parts = request_line.split_whitespace();
    let (status, body) = match (parts.next(), parts.next()) {
        (Some("GET"), Some("/user")) => ("200 OK", r#"{"login":"baker"}"#),
        _ => ("404 Not Found", r#"{"message":"Not Found"}"#),
    };
    let _ = write!(
        &stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}
