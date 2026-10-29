//! Running a tool's install command in a terminal window of its own. The command is a shell line (pipes, `$(…)`,
//! `&&`), so every runner hands it to a shell: `cmd.exe /c` in a plain console, `bash -lc` in mintty, `sh -c` on Linux.
use fleet_api::Os;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Git for Windows' own installer cannot replace the bash and mintty that would be running it, so it goes through a
/// plain console.
fn is_git_install(command: &str) -> bool {
    command.contains("Git.Git")
}

/// What to spawn to run `command` in a window that stays open until Enter (Linux) or a key (Windows console).
/// `mintty` is Git for Windows' terminal when present; `linux_terminal` is a terminal argv ending in its run flag.
/// `None` when Linux has no terminal.
pub fn install_argv(
    command: &str,
    os: Os,
    mintty: Option<&Path>,
    linux_terminal: Option<Vec<String>>,
) -> Option<Vec<String>> {
    match os {
        Os::Windows => {
            if let Some(mintty) = mintty.filter(|_| !is_git_install(command)) {
                let mut argv = crate::login::mintty_argv(mintty);
                argv.extend([
                    mintty.with_file_name("bash.exe").display().to_string(),
                    "-lc".into(),
                    format!("{command}; read -p \"Press Enter to close\""),
                ]);
                return Some(argv);
            }
            Some(vec![
                "cmd.exe".into(),
                "/c".into(),
                format!("{command} & pause"),
            ])
        }
        Os::Linux => {
            let mut argv = linux_terminal?;
            argv.extend([
                "sh".into(),
                "-c".into(),
                format!("{command}; printf \"\\nPress Enter to close\"; read _"),
            ]);
            Some(argv)
        }
    }
}

/// The Linux terminal to run an install in, made to stay until the install ends: gnome-terminal hands its command to
/// a server and returns at once, so the recheck that follows the install would run before it; `--wait` makes it
/// return when the command does. `x-terminal-emulator` is often gnome-terminal behind Debian's wrapper, which
/// `resolve` (the program's real file, links followed) shows; the wrapper's own `-e` cannot pass `--wait`, so
/// gnome-terminal beside it is run instead. Other terminals already wait for their command.
pub fn waiting_terminal(
    mut term: Vec<String>,
    resolve: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<String> {
    let Some(program) = term.first().cloned() else {
        return term;
    };
    let real = resolve(&program).unwrap_or_else(|| PathBuf::from(&program));
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.starts_with("gnome-terminal") {
        return term;
    }
    if name != "gnome-terminal" {
        term[0] = real.with_file_name("gnome-terminal").display().to_string();
    }
    // The run flag (`--` or `-e`) goes; `--wait --` takes its place.
    term.pop();
    term.extend(["--wait".to_string(), "--".to_string()]);
    term
}

/// `program`'s real file on the `PATH`, links followed: what `waiting_terminal` asks.
pub fn real_program(program: &str) -> Option<PathBuf> {
    std::fs::canonicalize(which::which(program).ok()?).ok()
}

/// The one argument after `cmd.exe /c`: the whole command in a pair of quotes, which cmd strips. Pass it with
/// `raw_arg`; Rust's own quoting would turn the command's inner quotes into `\"`, which cmd does not understand.
pub fn cmd_c_arg(command: &str) -> String {
    format!("\"{command}\"")
}

/// Windows: the child gets a console window of its own.
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// The process for `argv`, stdio untouched. The Windows console form (`install_argv`'s `cmd.exe /c <command> &
/// pause`) is built by hand: `/c` and the quoted command line go in raw, and cmd is run directly (not through
/// `start`, which would make a second cmd parse the quotes again). `new_console` gives it its own window; tests pass
/// false.
pub fn build_command(argv: &[String], new_console: bool) -> io::Result<Command> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no install command"))?;
    let mut cmd = Command::new(program);
    // The daemon's PATH: an installer that needs a tool installed before it finds it.
    fleet_fs::child::apply_path(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if program == "cmd.exe" && args.len() == 2 && args[0] == "/c" {
            cmd.raw_arg("/c").raw_arg(cmd_c_arg(&args[1]));
        } else {
            cmd.args(args);
        }
        if new_console {
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = new_console;
        cmd.args(args);
    }
    Ok(cmd)
}

/// Spawns `argv`; the caller waits on the child (on a thread) and then asks for a recheck.
pub fn run_install(argv: Vec<String>) -> io::Result<Child> {
    let mut cmd = build_command(&argv, true)?;
    // Windows: no stdio is set. Any explicit stdio makes CreateProcess hand the child exactly those handles, so with
    // `Stdio::null()` the new console window would show nothing and `pause` would read NUL and return at once.
    // Left alone, the new console (or mintty) supplies the handles.
    #[cfg(not(windows))]
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
}
