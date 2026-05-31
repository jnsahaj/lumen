# lumen ↔ coding agent integrations

Hook lumen into the review loop of your coding agent. **The agent never
runs lumen and never knows it exists** — lumen is wired into the
agent's stop event and injects your annotations as the next user
message in the conversation.

```
agent stops → lumen opens on the diff → annotate inline → press `s`
→ agent receives your feedback as if you typed it → agent fixes it
```

## How it works

Two primitives in lumen make this loop one-config-file per agent:

1. **`s` keybind** — in the diff TUI, `s` opens a confirmation modal.
   On `Enter`, lumen exits and writes the formatted annotations to
   stdout (the same text `y` copies to your clipboard).
2. **`--hook <protocol>` flag** — wraps stdout in the JSON envelope
   the agent's stop-hook expects, drains the event payload off stdin,
   and skips the TUI entirely when there's nothing to review. Today:
   `--hook codex-stop`.

Lumen's TUI also auto-routes to `/dev/tty` when stdout is captured, so
the alternate-screen escapes never pollute the JSON the agent reads.

## Agents

| Agent  | Status | Install |
|--------|--------|---------|
| [Codex](./codex/) | ✅ Works | `curl …/install.sh \| bash` (or manual) |
| [Pi](./pi/)       | ✅ Works | `pi install npm:@jnsahaj/lumen-pi-extension` |
| Claude Code | ⏳ Not yet | Same Stop-hook pattern as Codex, easy to add |

## Adding a new agent

If your agent has a stop hook that can synthesize a follow-up user
message (Codex `Stop`, Claude Code `Stop`, Gemini policy hooks, etc.):

1. Add a new variant to `HookFormat` in
   [`src/config/cli.rs`](../src/config/cli.rs).
2. Implement the JSON envelope in `emit_hook_response` in
   [`src/command/diff/app.rs`](../src/command/diff/app.rs).
3. Add a `hooks.json` (or equivalent) snippet under
   `integrations/<agent>/`.

If your agent only exposes hooks via a plugin API (Pi today), that's a
real extension package — open an issue first.
