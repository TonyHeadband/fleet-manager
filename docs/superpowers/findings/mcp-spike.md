HOLD (requires the three env vars below)

# Spike: Claude Code MCP over HTTP and held tool calls

Claude Code 2.1.310, run 2026-10-24 in a private tmux server against a stdlib
`http.server` MCP server (`wait(seconds)`, header `X-Fleet-Token: t1`). Scratch
files: `/tmp/claude-10133/-mnt-users-afrontino/71d72b47-a2c0-44b3-960a-645b566802ab/scratchpad/mcpspike/`
(`server.py`, `mcp.json`, `settings.json`, `hook.py`, `run.sh`). Nothing committed.

## Outcome

HOLD: a 1800 s (30 min) held `tools/call` works and returns the real result to the
agent, but only with three env vars on the `claude` process (a default config fails at 60 s):

```
MCP_TOOL_TIMEOUT=2400000                    # ms, hard cap on one call (default 60 s)
CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT=0         # 0 = no idle limit (default 300 s for http; the binary's code suggests 1800 s for stdio, never tested)
CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS=3600000  # default 120 s: after that the call is moved to a background task
```

Hold ceiling: about 40 min, implied by `MCP_TOOL_TIMEOUT=2400000`. Only 30 min was tested; nothing above that.
Set `MCP_TOOL_TIMEOUT` above the longest hold you want. The daemon must set all three in
the env of every agent it spawns. The three vars were only tested together (run 4 tested
`MCP_TOOL_TIMEOUT` alone, which was not enough); which of the other two is strictly needed is unknown.

## What a builder must apply

- Exact command line (run inside tmux, env as a prefix of the command string):
  `MCP_TOOL_TIMEOUT=2400000 CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT=0 CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS=3600000 ~/.local/bin/claude --mcp-config <dir>/mcp.json --settings <dir>/settings.json --permission-mode default -- '<prompt>'`
  (`run.sh` took the env prefix from `$EXTRA`). The daemon should set the same vars in the spawned process env.
- Permission: the long runs did not prompt because, in the first run, I chose "Yes, and don't ask again"
  which wrote `permissions.allow: ["mcp__fleet__wait"]` to `<workdir>/.claude/settings.local.json`.
  The daemon should instead put `permissions.allow` (`mcp__fleet__*`) in its generated `--settings` file. Not tested that way.
- Trust-folder dialog: it appears for a new cwd; I accepted it by selecting "Yes, I trust this folder"
  (Up, Enter in tmux) for the temp workdir only. The daemon has to cope with it for fresh project folders (not investigated).

## Findings

- **HTTP transport + header works.** `{"type":"http","url":...,"headers":{"X-Fleet-Token":"t1"}}`
  connected on the first try. The server logged the correct token on every POST. A wrong token returning 401 was not tested.
  It showed the tool as "fleet - Wait Tool (MCP)".
- **Request sequence the server saw** (one connection setup, then the call):
  1. `POST server/discover` (id `server-discover-probe-1`, protocolVersion `2026-07-28`). The spike server
     answered a JSON-RPC -32601 error and the CLI carried on, so unsupported is fine.
  2. `POST initialize` (id 0, protocolVersion `2025-11-25`, capabilities roots/elicitation)
  3. `POST notifications/initialized` (no id, answer 202)
  4. `GET /mcp` (SSE listen stream; the spike answered 405 and the CLI carried on)
  5. `POST tools/list` (id 1)
  6. `POST tools/call` (id 2), `_meta` includes `claudecode/toolUseId` (the `toolu_...` id, which equals the hook's `tool_use_id`)
  No `Mcp-Session-Id` was used (the spike never issued one). Plain `application/json` replies were enough, no SSE.
- **Tool name in hooks:** `PreToolUse` and `PostToolUse` report `tool_name: "mcp__fleet__wait"`,
  `tool_input: {"seconds": N}`, plus `mcp_server: {"name":"fleet","source":"dynamic"}`.
  `PostToolUse.tool_response` is the content array `[{"type":"text","text":"waited 1800s"}]`.
- **Hooks fired around the call:** `PreToolUse`, then `PostToolUse` on success or
  `PostToolUseFailure` on failure (payload `error: "The operation timed out."`). The agent first
  calls `ToolSearch` (its own Pre/Post pair, which returned `mcp__fleet__wait`), because the owner has
  many deferred MCP tools. Pre and the server's `tools/call` log line were within the same second
  (log resolution is 1 s), so their order is unknown. Post came when the reply arrived.
  Once, in the first run, a `PreToolUse` for `mcp__fleet__wait` with an `agent_id` and a new
  `tool_use_id` fired with no `tools/call` reaching the server (a background agent, "1 agent" in the
  status bar). So a Pre hook is not proof that a call reached the server.
- **Permission prompt:** the first call asks "Do you want to proceed?" (1 Yes, 2 Yes and don't ask
  again for fleet, 3 No). Whether `PreToolUse` fires before or after the permission answer was not established (the first run's
  hook log does not separate them). A trust-folder dialog appears first for a new cwd.

## Hold-duration results

The logs of runs 1-5 were overwritten by later runs (`run.sh` truncates them); this table is the only record of those runs.

| Run | Env | seconds | Result |
|---|---|---|---|
| 1 | none | 30 | OK, result returned (35 s total) |
| 2 | none | 120 | Fails at exactly 60 s: `PostToolUseFailure` "The operation timed out." Agent saw `is_error` tool_result "The operation timed out." Server's late reply hit `ConnectionResetError` (client closed the socket). |
| 3 | none | 90 | Same, failed at 60.0 s (02:13:47 to 02:14:47). |
| 4 | `MCP_TOOL_TIMEOUT=2000000` | 600 | Not a hold: at 120 s the call returned a text result "MCP tool "fleet/wait" is still running after 120s. It was moved to the background as task ... you'll receive a notification". `PostToolUse` fired at 02:02:37 with that text. At 300 s of silence the idle timeout aborted it; the agent was told "MCP task ... failed ... no response for 300s ... CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT". Server hit `ConnectionResetError` at 600 s. |
| 5 | all three vars (timeout 2400000) | 600 | HOLD OK: `PreToolUse` 02:15:02, `PostToolUse` 02:25:03 with `waited 600s`; one tool result, no background task. |
| 6 | all three vars | 1800 | HOLD OK: `PreToolUse` 02:25:33, `PostToolUse` 02:55:33 with `waited 1800s`. |

Server sent no bytes (no SSE keepalive or progress) during any hold, so the held request was
silent for the full 30 min and survived.

## What the agent sees

- Default (no vars): after 60 s an error tool_result `The operation timed out.`
- `MCP_TOOL_TIMEOUT` alone: at 120 s a "moved to background" text result (the agent can keep
  working and is later notified on completion), then a failure notice at 300 s idle.
- All three vars: the call blocks and returns the server's reply as one normal result.

## Implications for Task 7

- Use HOLD, with the three env vars injected by the daemon into every spawned agent.
- If any var is not honoured by a future Claude Code version, the fallbacks are visible: the 120 s
  "moved to background" notice and the 300 s idle failure. `request_repo` should treat a
  tool_result error or the background notice as "re-ask" rather than as a decision.
- Optional hardening not tried: per-server `timeout` in MCP settings (the idle-timeout message
  mentions it), and progress notifications.
