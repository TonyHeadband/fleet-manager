//! Running a tool's install command in a terminal window of its own. The command is a shell line (pipes, `$(…)`,
//! `&&`), so every runner hands it to a shell: `cmd.exe /c` in a plain console, `bash -lc` in mintty, `sh -c` on Linux.
use fleet_api::Os;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

const CONSOLE_TITLE: &str = "\"fleet-manager install\"";

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
                "start".into(),
                CONSOLE_TITLE.into(),
                "/wait".into(),
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

/// The one argument after `cmd.exe /c`: the whole command in a pair of quotes, which cmd strips. Pass it with
/// `raw_arg`; Rust's own quoting would turn the command's inner quotes into `\"`, which cmd does not understand.
pub fn cmd_c_arg(command: &str) -> String {
    format!("\"{command}\"")
}

/// The process for `argv`. The Windows console form (`install_argv`'s `cmd.exe /c start …`) is built by hand: its
/// title and its final `<command> & pause` go in raw, everything else is quoted normally.
pub fn build_command(argv: &[String]) -> io::Result<Command> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no install command"))?;
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let console = argv.len() == 8 && program == "cmd.exe" && argv[3] == CONSOLE_TITLE;
        for (i, a) in args.iter().enumerate() {
            match (console, i) {
                (true, 2) => cmd.raw_arg(a),
                (true, 6) => cmd.raw_arg(cmd_c_arg(a)),
                _ => cmd.arg(a),
            };
        }
    }
    #[cfg(not(windows))]
    cmd.args(args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(cmd)
}

/// Spawns `argv`; the caller waits on the child (on a thread) and then asks for a recheck.
pub fn run_install(argv: Vec<String>) -> io::Result<Child> {
    build_command(&argv)?.spawn()
}
