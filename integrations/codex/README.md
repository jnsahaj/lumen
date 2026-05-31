# lumen for Codex

Hook lumen into [OpenAI Codex](https://github.com/openai/codex)'s `Stop`
event. Codex finishes a turn → lumen pops up on the uncommitted diff
→ you annotate → press `s` → annotations are injected as the next user
message and Codex continues in the same turn.

```
agent stops → lumen opens → review → `s`
→ Codex sees your annotations as if you typed them → agent fixes them
```

**The agent never knows lumen exists.** It just receives feedback that
looks exactly like a user message.

## How it works

Codex `Stop` hooks can return:

```json
{ "decision": "block", "reason": "<text>" }
```

…which makes Codex *not* stop and instead inject `reason` as the next
user message. `lumen diff --hook codex-stop` handles this protocol:

| Lumen flow                       | Stdout                                            | Codex behavior      |
| -------------------------------- | ------------------------------------------------- | ------------------- |
| No uncommitted changes           | `{}` (TUI skipped entirely)                       | Turn ends normally  |
| User pressed `s`, sent feedback  | `{"decision":"block","reason":"<annotations>"}`   | Continues with text |
| User pressed `q`, dismissed      | `{}`                                              | Turn ends normally  |

## Install

One-shot script — detects Codex, flips the `hooks` feature flag in
`~/.codex/config.toml`, and writes `~/.codex/hooks.json`. Idempotent;
refuses to clobber an existing `hooks.json` (prints the snippet to add
manually instead).

```bash
curl -fsSL https://raw.githubusercontent.com/jnsahaj/lumen/main/integrations/install.sh | bash

# or from a clone:
bash integrations/install.sh
```

Restart Codex. Done.

### Manual install

If you'd rather not run a script:

`~/.codex/config.toml` (or repo-local `.codex/config.toml`):

```toml
[features]
hooks = true
```

`~/.codex/hooks.json` — copy [`hooks/hooks.json`](./hooks/hooks.json),
or paste:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "lumen diff --hook codex-stop",
            "timeout": 345600
          }
        ]
      }
    ]
  }
}
```

### Notes

- Use an absolute path to `lumen` (e.g. `/opt/homebrew/bin/lumen`) if
  you launch Codex Desktop — app-launched processes don't always
  inherit your shell `PATH`.
- `timeout` is in seconds; the large value lets you take your time
  reviewing without Codex killing the hook.
- Codex hooks are currently disabled on Windows in the official docs.

## Test it without Codex

The hook contract is `stdin = event JSON, stdout = response JSON`, so
you can drive it from a shell:

```bash
# Inside a repo with uncommitted changes:
echo '{"hook_event_name":"Stop"}' | lumen diff --hook codex-stop
```

- TUI opens (on `/dev/tty`, since stdout is captured).
- Annotate, press `s` → `Enter`.
- Stdout receives `{"decision":"block","reason":"# ...\n\n**file** line N\n\n...comment..."}`.
- Press `q` instead → stdout receives `{}`.

In a clean working tree the same command returns `{}` instantly without
opening the TUI.
