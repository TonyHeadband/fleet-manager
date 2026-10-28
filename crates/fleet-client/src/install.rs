//! Running a tool's install command in a terminal window of its own. The command is a shell line (pipes, `$(…)`,
//! `&&`), so every runner hands it to a shell: `cmd.exe /c` in a plain console, `bash -lc` in mintty, `sh -c` on Linux.
use fleet_api::Os;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

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
            Some(
                [
                    "cmd.exe",
                    "/c",
                    "start",
                    "\"fleet-manager install\"",
                    "/wait",
                    "cmd.exe",
                    "/c",
                ]
                .into_iter()
                .map(String::from)
                .chain([format!("{command} & pause")])
                .collect(),
            )
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

/// Spawns `argv`; the caller waits on the child (on a thread) and then asks for a recheck.
pub fn run_install(argv: Vec<String>) -> io::Result<Child> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no install command"))?;
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        // `start "title"` needs its title's quotes verbatim; the default argument quoting would escape them.
        use std::os::windows::process::CommandExt;
        for a in args {
            if a.len() > 1 && a.starts_with('"') && a.ends_with('"') {
                cmd.raw_arg(a);
            } else {
                cmd.arg(a);
            }
        }
    }
    #[cfg(not(windows))]
    cmd.args(args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}
