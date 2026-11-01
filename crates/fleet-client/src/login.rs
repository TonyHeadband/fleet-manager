//! Logging a persona in: a terminal running the real `claude` / `gh` CLI. fleet never reads the token the CLI
//! stores; it only starts the CLI against the persona's own config folder.
use fleet_api::{ambient_vars, Persona};
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Quotes `s` for a POSIX shell. A leading `~` or `~/` stays outside the quotes because the shell does not expand a
/// quoted tilde; that only works where the shell expands it (a command word such as `cd ~/x`, not `VAR=~/x` in dash),
/// so pass absolute paths there.
pub fn shell_quote(s: &str) -> String {
    let (head, rest) = if s == "~" {
        ("~", "")
    } else if let Some(r) = s.strip_prefix("~/") {
        ("~/", r)
    } else {
        ("", s)
    };
    let plain = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-+@%:,=".contains(c));
    if head.is_empty() && rest.is_empty() {
        return "''".into();
    }
    let quoted = if plain || rest.is_empty() {
        rest.to_string()
    } else {
        format!("'{}'", rest.replace('\'', "'\\''"))
    };
    format!("{head}{quoted}")
}

/// `argv` as one shell command line: every word quoted, joined with spaces.
pub fn shell_join<S: AsRef<str>>(argv: &[S]) -> String {
    argv.iter()
        .map(|a| shell_quote(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The terminals tried in order, each with the flag that makes it run the rest of its argv as a command.
const TERMINALS: [(&str, &str); 5] = [
    ("x-terminal-emulator", "-e"),
    ("gnome-terminal", "--"),
    ("konsole", "-e"),
    ("xfce4-terminal", "-x"),
    ("xterm", "-e"),
];

/// The terminal to open a login in: `$TERMINAL` when it exists, else the first known one on the PATH. `$TERMINAL`
/// is split on whitespace: the first word is the program, the rest are its arguments, placed before `-e`. A
/// terminal that re-parses what follows `-e` as one string is not supported.
pub fn terminal_argv(
    env_terminal: Option<&str>,
    on_path: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    if let Some(words) = env_terminal.map(|t| t.split_whitespace().collect::<Vec<_>>()) {
        if words.first().is_some_and(|w| on_path(w)) {
            let mut argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            argv.push("-e".to_string());
            return Some(argv);
        }
    }
    TERMINALS
        .iter()
        .find(|(name, _)| on_path(name))
        .map(|(name, flag)| vec![name.to_string(), flag.to_string()])
}

/// `name` is an executable on the `PATH`, with `PATHEXT` on Windows.
pub fn on_path(name: &str) -> bool {
    which::which(name).is_ok()
}

/// Git for Windows' terminal. ConPTY mode gives native console programs (`claude`) a real console; without it they
/// see a pipe and refuse to run interactively.
pub fn mintty_argv(mintty: &Path) -> Vec<String> {
    vec![
        mintty.display().to_string(),
        "-o".into(),
        "ConPTY=true".into(),
        "-e".into(),
    ]
}

/// The terminal to open a command in on this machine: on Windows the mintty that comes with Git for Windows,
/// elsewhere per `$TERMINAL` and the `$PATH`.
pub fn terminal() -> Option<Vec<String>> {
    if cfg!(windows) {
        let bash = fleet_fs::posix_shell()?;
        return fleet_fs::shell::mintty_beside_bash(&bash).map(|m| mintty_argv(&m));
    }
    terminal_argv(std::env::var("TERMINAL").ok().as_deref(), on_path)
}

/// The URL's host is this machine: an agent's terminal is reachable from here only then.
pub fn is_loopback_url(base_url: &str) -> bool {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `env -u VAR …` for every ambient variable: a login must not be answered by the user's own token or API key.
fn env_prefix() -> String {
    let unset: String = ambient_vars()
        .map(|v| format!("-u {} ", shell_quote(v)))
        .collect();
    format!("env {unset}")
}

/// A program path as a command word for bash: Git Bash on Windows takes `C:/x/y.exe` (a path, by its `/`) but would
/// look `C:\x\y.exe` up as a name. Unchanged elsewhere.
fn script_program(program: &str) -> String {
    if cfg!(windows) {
        program.replace('\\', "/")
    } else {
        program.to_string()
    }
}

/// Starts Claude in the tasks root, so the first screen is the trust dialog (once per persona); the user then
/// types `/login`. Whether the root is already trusted is in Claude's files, which fleet does not read.
pub fn claude_login_script(p: &Persona, claude: &[String], tasks_root: &str) -> String {
    let claude: Vec<String> = claude
        .iter()
        .enumerate()
        .map(|(i, a)| match i {
            0 => shell_quote(&script_program(a)),
            _ => shell_quote(a),
        })
        .collect();
    format!(
        "cd {} && {}CLAUDE_CONFIG_DIR={} {}",
        shell_quote(tasks_root),
        env_prefix(),
        shell_quote(&p.claude_dir),
        claude.join(" ")
    )
}

/// Both gh logins pass `--insecure-storage`: the token goes to `<gh_dir>/hosts.yml` (the folder is 0700), not the
/// system keyring. The keyring is shared across personas and the user's own gh, and an agent's `git push` could pop
/// an unlock window.
const GH_STORAGE_FLAG: &str = "--insecure-storage";

/// Runs gh's interactive login for the persona's host. `GIT_CONFIG_GLOBAL` points into the persona's folder because
/// gh offers to register itself as git's credential helper with `git config --global`, which must not touch the
/// user's own `~/.gitconfig`.
pub fn gh_login_script(p: &Persona, gh: &str) -> String {
    let gitconfig = format!("{}/gitconfig", p.gh_dir.trim_end_matches('/'));
    format!(
        "{}GH_CONFIG_DIR={} GIT_CONFIG_GLOBAL={} {} auth login --hostname {} --git-protocol https {}",
        env_prefix(),
        shell_quote(&p.gh_dir),
        shell_quote(&gitconfig),
        shell_quote(&script_program(gh)),
        shell_quote(&p.host),
        GH_STORAGE_FLAG
    )
}

/// What the terminal runs: the script, then a pause so the window outlives a fast failure.
/// Public so tests can pin the exact argv. Fails when there is no POSIX shell (Windows without Git for Windows).
pub fn terminal_command(term: &[String], script: &str) -> io::Result<Vec<String>> {
    let shell = fleet_fs::posix_shell().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Git for Windows (bash.exe) not found",
        )
    })?;
    let mut argv = term.to_vec();
    argv.extend([
        shell.display().to_string(),
        "-c".to_string(),
        format!("{script}; echo; printf \"Done. Press Enter to close.\"; read _"),
    ]);
    Ok(argv)
}

/// Opens `script` in a terminal and returns at once; the terminal outlives this process.
pub fn open_in_terminal(term: &[String], script: &str) -> io::Result<()> {
    let argv = terminal_command(term, script)?;
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no terminal command"))?;
    let mut cmd = Command::new(program);
    // The daemon's PATH, so what the script runs finds what the daemon found.
    fleet_fs::child::apply_path(&mut cmd);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        // Its own process group, so closing the GUI's terminal or Ctrl-C does not take the login with it.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    // Reaped off-thread: a terminal that stays in the foreground would otherwise leave a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
