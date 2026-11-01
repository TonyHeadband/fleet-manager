# Claude Code transcript fixtures

Real session transcripts (`~/.claude/projects/<encoded cwd>/<session id>.jsonl`) captured from **Claude Code
2.1.316** on Linux, model `claude-haiku-5-5`, on 2026-10-31. Every session ran as the interactive TUI in a PTY
(120x45, `TERM=xterm-256color`), driven by a script that sent key bytes. Each session got a `--settings` file shaped
like `hooks_cfg.rs` writes it: an `http` hook for every event in `HTTP_EVENTS` pointing at a local Python server that
logged each body, and `"tui": "default"`.

The later sections record what was learned about answering permission prompts and `AskUserQuestion`. Each finding is
marked **CONFIRMED** (seen in a run) or **UNCONFIRMED** (inferred).

## Fixtures

| File | Case | How it was made |
| --- | --- | --- |
| `tools.jsonl` | Read, a search, Edit, and a failing Bash in one turn | Rules allowed `Read`, `Edit`, `Bash`. The prompt asked for each step as a separate call. |
| `clear-before.jsonl` | `/clear`: the first session file | The same session as `tools.jsonl`, copied after `/clear`, so it ends with the `cost-state` line that `/clear` writes. |
| `clear-after.jsonl` | `/clear`: the second session file | The new session id that `/clear` starts. It opens with the `/clear` command lines, then one Read turn. |
| `compact.jsonl` | Manual compaction | `clear-after`'s session after `/compact` and one more prompt. Compaction stays in the same file. |
| `ask-question.jsonl` | `AskUserQuestion` answered in the TUI | Five questions: a digit, arrows and Enter, a multi-select, "Type something" with free text, and a two-question form (single choice with free text, multi-select with free text). |
| `permission.jsonl` | Permission prompts, allowed and denied | TUI **Yes** (`1`), TUI **No** (`4`), then hook decisions: allow, deny, deny with `interrupt`, allow-for-session, and TUI answers while a hook was held. |
| `permission-hook.jsonl` | Hook timeout, dropped connection, `AskUserQuestion` answered by the hook | PermissionRequest hook with `"timeout": 15`. |
| `mcp.jsonl` | MCP tool call | A tiny stdio MCP server `calc` with one tool `add`, passed with `--strict-mcp-config --mcp-config`. The rule allowed `mcp__calc__add`. The call goes `ToolSearch` → `mcp__calc__add`. |

### Scrubbing and trimming

- Home paths and the username became `/home/user` and `user`. The temp folders `/tmp/claude-1000/cp-<case>` became
  `/tmp/fixture/<case>`, including the encoded project folder name. No email addresses appeared.
- Dropped lines: `api-request`, `api-request-blob`, `api-request-shape`. These hold the full system prompt and the
  owner's environment. None of them has a `uuid`, so no `parentUuid` chain breaks.
- Lines kept with their content redacted: the attachments below. Their `attachment` became
  `{"type":"<type>","redacted":true}`, and each `rendered[].content` became `"[redacted]"`. All other fields, including
  `uuid` and `parentUuid`, are unchanged.
  - `agent_listing_delta`
  - `deferred_tools_delta`
  - `hook_additional_context`
  - `hook_success`
  - `instructions`
  - `mcp_instructions_delta`
  - `prompt_snapshot`
  - `remote_session_change`
  - `session_context`
  - `skill_listing`
  - `credential_org`
  - `environment`

  Those lines carried the owner's CLAUDE.md, skills, plugins, MCP servers and organization id.
- Every other line is byte-for-byte the CLI's output, apart from the path substitutions above.
- The owner's plugins were active during capture, which is where the `hook_success` and `hook_additional_context`
  attachments and `stop_hook_summary` come from. A daemon-spawned agent will also get some.

### Parser notes

- Transcript line types seen:
  - `user`
  - `assistant` (each content block is a separate line: `thinking`, `text`, and `tool_use` arrive one per line under
    the same `message.id`)
  - `system`, with subtypes `stop_hook_summary`, `turn_duration`, `local_command`, `compact_boundary`
  - `attachment` (many types)
  - `last-prompt`
  - `mode`
  - `permission-mode`
  - `atis-latch`
  - `ai-title`
  - `tools-used`
  - `file-history-snapshot`
  - `file-history-delta`
  - `cost-state`
- **There is no `Grep` or `Glob` tool in 2.1.316.** The tool list is `Agent, Artifact, AskUserQuestion, Bash, Edit,
  Read, Skill, ToolSearch, Write, …`. A search is a `Bash` call (`grep -rn …`). The TUI collapses it into "Searched for
  1 pattern".
- A tool result's `user` line carries `permissionDecision`:
  - `{decision:"accept", source:"config"|"user_temporary"|"hook", grantScope:"once"|"session", reasonCode, …}`
  - On a deny, `{decision:"reject", source:"hook", reasonCode:"hook:PermissionRequest"}` plus
    `toolDenialKind:"permission-rule"`.
  - A TUI **No** (or a hook deny with `interrupt`) gives `toolDenialKind:"user-rejected"`, an `is_error` tool_result
    "The user doesn't want to proceed…", and then a `user` line that ends the turn.
- A hook decision also writes an `attachment` of type `hook_permission_decision` (`decision`, `toolUseID`), just before
  the tool result.
- `AskUserQuestion`'s structured answers are in the tool-result line's `toolUseResult.answers`, keyed by question text.
  A multi-select answer is the chosen labels joined with `", "`. Free text appears verbatim.
- `/clear` starts a new session file. Its first chat lines are an `isMeta` `<local-command-caveat>` user line, a
  `<command-name>/clear</command-name>` user line, and a `system`/`local_command` line.
- Compaction writes `system`/`compact_boundary` (`compactMetadata.trigger:"manual"`, `preTokens`, `postTokens`, …).
  Then comes a `user` line with `isCompactSummary:true, isVisibleInTranscriptOnly:true` (the summary), then the
  `/compact` command lines.
- A hook that cannot be reached writes `attachment` `hook_non_blocking_error` (`stderr: "connect ECONNREFUSED …"`).
  `permission-hook.jsonl` line 59 has one from a moment when the test server was down.

## Answering a permission prompt from a PermissionRequest hook

The `http` hook's response body decides. Entry used: `{"type":"http","url":…,"headers":…,"allowedEnvVars":…,"timeout":N}`.

| Decision | Response body | Status |
| --- | --- | --- |
| Allow once | `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}` | **CONFIRMED.** The tool runs, and the next identical call prompts again. |
| Allow for the session | `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"python3:*"}],"behavior":"allow","destination":"session"}]}}}` | **CONFIRMED.** The next `python3 …` call fired no PermissionRequest at all, and nothing was written to disk. |
| Deny (Claude carries on) | `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied by the fleet owner from the chat."}}}` | **CONFIRMED.** `message` becomes the `is_error` tool result. Claude replies and the turn ends normally with `Stop`. |
| Deny and stop the turn | the same body with `"interrupt":true` | **CONFIRMED.** Same as TUI **No**: "Interrupted · What should Claude do instead?". **No `Stop` hook fires.** |
| No decision | an empty `200` body | **CONFIRMED.** Claude keeps its own TUI prompt. |

- **Suggested rules for "allow for session".** The PermissionRequest body includes `permission_suggestions`. For
  `python3 -c "print(42)"` it was
  `[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"python3 -c \"print(42)\""}],"behavior":"allow","destination":"localSettings"}]`,
  which is what TUI option 2 "don't ask again" would save. To scope it to the session, send the suggestion back with
  `"destination":"session"`. The test above used a hand-written `python3:*` rule in the same shape, and both are
  **CONFIRMED** to be accepted.
- **Folder-scoped suggestions under `/tmp/claude-1000`.** Here the suggestion was
  `{"type":"addDirectories","directories":[cwd],"destination":"session"}` (plus `setMode acceptEdits` for `mkdir`). It
  never stopped the next prompt, whether the hook returned it or the TUI's own option 2 chose it. That folder is
  Claude's own temp root (`scratchpad_dir` lives there), which looks special-cased. **UNCONFIRMED** in a normal project
  folder.

### How long a hook may hold

- **CONFIRMED.** The `timeout` field on the hook entry is in seconds.
  - With `"timeout": 15`, an answer at 12 s was applied.
  - An answer at 40 s was ignored, and the TUI prompt stayed up for the owner.
- **CONFIRMED.** Without `timeout`, a 100 s hold was applied.
- The default for an `http` hook is **600 s**. **UNCONFIRMED** by a run. It was read from the 2.1.316 bundle:
  `let L=e.timeout?e.timeout*1000:h` with a default of `600000`. There is no upper cap in that code path.

### What the TUI does while the hook holds

- **CONFIRMED.** The prompt shows in the TUI at the same moment PermissionRequest is sent, not after the hook gives
  up. Both answer paths race, and the first one wins:
  - A TUI answer while the hook was held ran the tool, and the hook's later reply was ignored.
  - A hook answer while the TUI prompt was on screen closed the prompt.
- **CONFIRMED.** `Notification` (`notification_type: "permission_prompt"`) fires about 6 s after the prompt opens if
  nothing has answered, even while a hook is still held.
- **CONFIRMED.** Hook timeout: the TUI prompt simply stays. There is no error line, and the tool waits for a TUI answer.
- **CONFIRMED.** Connection dropped (server closed the socket after 5 s without a response): the same, the TUI prompt
  stays.
- **CONFIRMED.** A TUI **No** fires no `Stop`, `PostToolUseFailure` or `PermissionDenied` hook, and neither does a
  hook deny with `interrupt`. The only trace is the transcript.

## AskUserQuestion

- **CONFIRMED.** `AskUserQuestion` fires `PreToolUse` and then **`PermissionRequest`** (`tool_name: "AskUserQuestion"`,
  `tool_input.questions`). A held PermissionRequest hook therefore also catches questions.
- **Answering a question from the hook. CONFIRMED.** Reply `allow` with `updatedInput` set to the original
  `tool_input` plus `answers`. This works for one question and for a two-question form (single choice plus
  multi-select), and no TUI dialog is shown:

  ```json
  {"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow",
    "updatedInput":{"questions":[…as received…],"answers":{"Which color do you like?":"Green",
                                                          "Which fruits do you like?":"Apple, Cherry"}}}}}
  ```

  - Keys are the question texts.
  - A multi-select answer is the labels joined with `", "`.
  - Free text is accepted as-is (`"Purple"`, not an option label).
- **CONFIRMED.** A plain `allow` without `updatedInput.answers` still shows the TUI question dialog.

### Answering from the PTY

The menu lists the options as 1..n, then "n+1. Type something", then "Chat about this". Keys are sent as bytes:

| Action | Keys | Status |
| --- | --- | --- |
| Single choice, option N (one question) | the digit `N`. It selects and submits at once, with no Enter. | **CONFIRMED** |
| Single choice by arrows | `ESC [ B` (Down) × (N-1), then `\r` | **CONFIRMED** |
| Single choice, "Type something" with free text | the digit `n+1`, which moves the cursor onto the row and makes it a text field; type the text; `\r`. Text starting with a digit works. | **CONFIRMED** |
| Multi-select, toggle option N | the digit `N` toggles `[✔]` **without moving the cursor**. Repeat for each option. | **CONFIRMED** |
| Multi-select, submit (one question) | `ESC [ C` (Right) to the "Submit" tab, which shows "Review your answers"; then `1` (or `\r`) for "Submit answers" | **CONFIRMED** (`1`; `\r` used on the two-question form) |
| Multi-select, "Type something" with free text | the digit `n+1` only toggles the box and does not focus the field (typed text is lost). Instead move with `ESC [ B` × n onto the row, type the text, then `\t` to the "Submit" row and `\r`, which goes to the review screen; then `\r` on "Submit answers". | **CONFIRMED** (the box had been ticked by the digit first; whether typing alone ticks it is **UNCONFIRMED**) |
| Several questions | Each answered question moves on to the next tab. A single-choice digit, or `\r` after free text, advances. The last step is the review screen and its "1. Submit answers". `Tab`, `←` and `→` move between tabs. | **CONFIRMED** for a two-question form (single choice with free text, then multi-select) |
| Enter on a multi-select option row | probably toggles it | **UNCONFIRMED**, not tried |
| `Esc` | cancels the question ("Esc to cancel") | **UNCONFIRMED**, not tried |
