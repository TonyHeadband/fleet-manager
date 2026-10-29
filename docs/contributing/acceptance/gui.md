# Release acceptance checklist

Run before tagging a release, from the release archives, against the real `claude`, on each system the release
supports: **Windows 11** (and Windows 10 if at hand), **Rocky 8** (glibc 2.28, the floor) and **Nobara or Fedora**
(the newest). Click-level behaviour is covered by the UI tests (`cargo test -p fleet-gui`); this checks what only a
real machine shows. Use a fresh user account or a VM where you can, so nothing from a development setup helps.

## Archive

- [ ] The archive unpacks to one `fleet-manager/` folder holding `fleet-gui`, `fleet-daemon`, `fleet-attach`
  (`.exe` on Windows), `README.txt`, `LICENSE` and `NOTICE`.
- [ ] Windows: double-clicking `fleet-gui.exe` opens the window with no console window behind it. If SmartScreen
  warns, More info → Run anyway works, as `README.txt` says. The taskbar shows the Fleet Manager icon, and the
  file's Properties → Details show its version.
- [ ] Linux: `./fleet-gui` starts on Rocky 8 with no "GLIBC_2.xx not found" error.

## Start and setup wizard

- [ ] With no config, the logo and progress bar show while the daemon starts and checks the tools, then **Step 1
  of 4 · Where Fleet Manager lives** with the default folders.
- [ ] **What Fleet Manager needs** lists Git (and Git Bash on Windows), Claude Code and GitHub CLI with what the
  daemon found. On a machine missing a tool: **Install** opens a terminal window running the shown command, and
  on Linux it asks for your password there; **Copy** copies it; the link opens the download page.
- [ ] After an install finishes, **Check again** finds the tool with no restart of the app (the daemon reads the
  new `PATH`). A too-old `gh` (below 2.40) shows as too old with its version.
- [ ] **Continue without it** goes on; the last step names what is still missing.
- [ ] Your first persona, then **Log in**: **Claude** opens a terminal (Git Bash's mintty on Windows) running
  `claude` with the persona's config folder; after "Yes, I trust this folder" and `/login`, the line turns green
  with the account. GitHub with **gh login** and with a **Personal access token** both turn green. **Finish** is
  enabled only when both are.
- [ ] With **Add a Start menu shortcut** (Windows) or **Add an app menu shortcut** (Linux) ticked, Finish adds a
  **Fleet Manager** shortcut that starts the app. Settings → General → Shortcuts shows it; Remove shortcuts
  removes it; moving the folder makes it "point to an old location" with Update.
- [ ] Browse… next to each folder field opens the system folder picker with no console window flashing.

## Tasks and the board

- [ ] Create a product with a path repo and a URL repo, then **New task** with both: the card appears, the agent
  goes Working, then Needs you with a "Permission needed" badge (with `claude_command` in manual permission mode, see
  [First run](../../getting-started/first-run.md#permission-mode)); answering from its card returns it to Working,
  then Review with "Has changes".
- [ ] The side panel's terminal follows the agent and is read-only; the Diff tab lists each repo's changes.
- [ ] **Open in terminal** opens a terminal attached to the agent (`fleet-attach`): typing reaches `claude`,
  Ctrl-] detaches, and the agent keeps running.
- [ ] Stop, then Confirm Stop, moves the card to Stopped with "stopped by you", and ends every process the agent
  started (no `claude`, shell or test runner left in Task Manager or `ps`). The task folder, worktrees and branches
  stay. Archive… then removes the worktrees.
- [ ] Killing an agent's `claude` process by hand moves its card to Stopped with a red "Crashed" badge.
- [ ] An agent opens a pull request on GitHub with the persona's own login, never yours.

## Closing, restart and resume

- [ ] Closing with agents running shows the dialog with the real counts. **Keep the daemon running** closes only
  the window: the agents keep working, and reopening the app shows them where they were.
- [ ] **Stop the daemon too** ends the daemon (no `fleet-daemon` process left) and its agents. The next start
  resumes them: each card comes back with "resumed after daemon restart" and its conversation intact.
- [ ] "Remember my choice" skips the dialog next time; Settings → General → Closing the window shows the saved
  choice, and Ask brings the dialog back.
- [ ] Relaunch setup wizard (Settings → General), change the tasks folder, Next: Done offers **Restart now**;
  with an agent running it asks first. After the restart, General shows the new folder and the next task lands
  there.

## Update

- [ ] With an older version running and a newer release published (or `FLEET_RELEASES_URL` pointing at a test
  feed), the update notice shows on start; **Download** opens the release page; **Skip this version** stops it
  for that version only. Settings → General → Version shows the newer one.
- [ ] Unpacking the new archive over the old folder (after Stop the daemon too) keeps the settings, personas and
  tasks. If an old daemon is still running, the app offers to restart it ("The background service is from …").

## Known limits

- Claude Code only, loopback only, no authentication.
- No tray: closing the window either keeps the daemon running in the background or stops it (#343).
- Characters the bundled fonts lack show as boxes in the terminal preview.
- macOS is not a release target yet; CI reports it without blocking.
