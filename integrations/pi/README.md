# lumen for Pi

Hook lumen into the [Pi coding agent](https://github.com/earendil-works/pi):

```
/lumen-diff → lumen opens on the diff → annotate inline
→ press `s` → annotations injected as Pi's next user message
→ agent fixes them
```

**The agent never invokes lumen.** Pi runs it from the `/lumen-diff`
slash command (or, opt-in, from `agent_end`), suspends its TUI while
lumen owns the terminal, then injects your annotations via
`pi.sendUserMessage()` so they appear exactly as if you typed them.

## Install

A small TypeScript extension lives in [`./extension/`](./extension/).
Symlink it into Pi's extensions dir:

```bash
mkdir -p ~/.pi/agent/extensions
ln -s "$(pwd)/integrations/pi/extension" ~/.pi/agent/extensions/lumen
```

Or run once without installing:

```bash
pi -e "$(pwd)/integrations/pi/extension/index.ts"
```

Full install docs (incl. post-npm-publish path) and config knobs in
[`extension/README.md`](./extension/README.md).

## How it works

| Trigger               | Extension behavior                                                                          |
|-----------------------|---------------------------------------------------------------------------------------------|
| Slash `/lumen-diff`   | Suspend Pi's TUI, run `lumen diff`, restart Pi's TUI. Forwards args to `lumen diff`.        |
| `agent_end` (opt-in: `LUMEN_AUTO_REVIEW=1`) | Same flow, automatically. Skip if no uncommitted changes.                |
| `lumen diff` exits, stdout non-empty | Call `pi.sendUserMessage(stdout)` → agent runs another turn with the annotations as input.   |
| `lumen diff` exits, stdout empty     | Do nothing — user pressed `q`, just looked.                                                  |

The TUI handoff uses the same `ctx.ui.custom` pattern Pi's own
[`interactive-shell` example](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/interactive-shell.ts)
uses for vim / htop / etc., so it composes cleanly with Pi's terminal
ownership.

## Why not a config file like Codex?

Pi's stop-equivalent hook is only available through the extension API
(`pi.on("agent_end", ...)` + `pi.sendUserMessage(...)`). Unlike Codex's
JSON-config `Stop` hook, you can't get this from a static file — it has
to be loaded code. The good news: jiti makes Pi run TS extensions
directly, no build step.
