# @jnsahaj/pi-lumen-diff

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
pi install npm:@jnsahaj/pi-lumen-diff
```

## Usage

Run `/lumen-diff` whenever you want to review the working-tree diff.
Annotate, press `s` → `Enter`. The agent gets your feedback as its next
prompt.

```
/lumen-diff
/lumen-diff HEAD~1
/lumen-diff main..-
/lumen-diff --file src/auth.rs
```

To also pop lumen up automatically after every agent turn, set
`LUMEN_AUTO_REVIEW=1`.

## Config

| Env var              | Default  | Meaning                                                          |
|----------------------|----------|------------------------------------------------------------------|
| `LUMEN_BIN`          | `lumen`  | Path to the lumen binary (use absolute if not on `$PATH`).       |
| `LUMEN_AUTO_REVIEW`  | `0`      | Set to `1` to pop lumen up after every `agent_end`.              |
