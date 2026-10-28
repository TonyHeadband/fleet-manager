//! fleet-attach [--url <daemon url>] <agent id>: takes over an agent's terminal. Ctrl-] detaches.
use fleet_client::attach::{connect, session, Outcome};

fn fail(e: String) -> ! {
    eprintln!("fleet-attach: {e}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut base = "http://127.0.0.1:7878".to_string();
    let mut id = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--url" => base = args.next().unwrap_or_default(),
            _ => id = Some(a),
        }
    }
    let Some(id) = id else {
        eprintln!("usage: fleet-attach [--url <daemon url>] <agent id>");
        std::process::exit(2);
    };
    // The banner waits for the daemon to accept, so a refusal is the only line printed.
    let ws = connect(&base, &id).unwrap_or_else(|e| fail(e));
    eprintln!("attaching to {id}; Ctrl-] detaches");
    match session(ws) {
        Ok(Outcome::Detached) => eprintln!("\r\ndetached"),
        Ok(Outcome::Exited(_)) => eprintln!("\r\nagent exited"),
        Err(e) => fail(e),
    }
}
