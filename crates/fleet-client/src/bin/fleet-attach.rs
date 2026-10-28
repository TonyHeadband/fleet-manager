//! fleet-attach [--url <daemon url>] <agent id>: takes over an agent's terminal. Ctrl-] detaches.
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
    eprintln!("attaching to {id}; Ctrl-] detaches");
    match fleet_client::attach::run(&base, &id) {
        Ok(fleet_client::attach::Outcome::Detached) => eprintln!("\r\ndetached"),
        Ok(fleet_client::attach::Outcome::Exited(_)) => eprintln!("\r\nagent exited"),
        Err(e) => {
            eprintln!("fleet-attach: {e}");
            std::process::exit(1);
        }
    }
}
