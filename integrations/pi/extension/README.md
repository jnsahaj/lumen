# lumen-pi-extension

A [Pi coding agent](https://github.com/earendil-works/pi) extension that
hooks lumen into the review loop:

```
agent finishes turn → lumen opens on the diff → annotate inline
→ press `s` → annotations injected as next user message → agent fixes them
```

The agent never invokes lumen. Pi runs it from the `agent_end` event,
suspends its own TUI while lumen owns the terminal, then injects the
annotations via `pi.sendUserMessage()` so they appear as if the user
typed them.

## Install

Requires lumen ≥ 2.25 on `$PATH` (or set `LUMEN_BIN`).

**From source (recommended while iterating):**

```bash
git clone https://github.com/jnsahaj/lumen.git
mkdir -p ~/.pi/agent/extensions
ln -s "$(pwd)/lumen/integrations/pi/extension" ~/.pi/agent/extensions/lumen
```

**Once-off try without installing:**

```bash
pi -e $(pwd)/lumen/integrations/pi/extension/index.ts
```

**From npm (post-publish):**

```bash
pi install npm:lumen-pi-extension
```

## Usage

Just run Pi. After every agent turn that produced uncommitted changes,
lumen pops up. Annotate, press `s` → `Enter`. The agent gets your
feedback as its next prompt.

Manual trigger (e.g. to re-review or scope to a range):

```
/lumen-review
/lumen-review HEAD~1
/lumen-review main..-
/lumen-review --file src/auth.rs
```

## Config

| Env var              | Default  | Meaning                                                          |
|----------------------|----------|------------------------------------------------------------------|
| `LUMEN_BIN`          | `lumen`  | Path to the lumen binary (use absolute if not on `$PATH`).       |
| `LUMEN_AUTO_REVIEW`  | `1`      | Set to `0` to disable the `agent_end` auto-trigger.              |

With `LUMEN_AUTO_REVIEW=0`, the extension only fires when you run
`/lumen-review` explicitly.
