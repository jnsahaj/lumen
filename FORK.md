# lumiere

This repository is a fork of [jnsahaj/lumen](https://github.com/jnsahaj/lumen) for
reviewing agent-written changes: several repositories from one workspace folder, and
a review loop with a coding agent.

- `origin`: https://github.com/aduverger/lumiere.git
- `upstream`: https://github.com/jnsahaj/lumen.git

## What differs from upstream

- The binary is named `lumiere`, so it can be installed beside upstream `lumen`. The
  Cargo package name, the configuration file (`lumen.config.json`), its locations and
  the `LUMEN_*` environment variables are unchanged.
- `A...-` is a new reference form: from the merge base of `A` and the current commit
  to the working tree, uncommitted changes included. `lumiere diff origin/main...-`
  shows everything a branch would add, even after `origin/main` has moved on.
- `lumiere diff` works from any directory of a repository; files, watch mode and the
  editor are resolved from the repository root.
- `j` means up or previous and `k` means down or next, in the diff, the sidebar and
  every list. This is the reverse of upstream.
- `Option+Down` or `Option+K` opens the next file, `Option+Up` or `Option+J` the
  previous one (`Alt` elsewhere). `Ctrl+K` and `Ctrl+J` do the same.
- Workspace reviews, review threads answered by a coding agent, and the `/lumiere` pi
  command, described below.
- When stdout is captured, the theme is no longer detected by querying the terminal,
  because the query was written into the captured annotations. Set `--theme`,
  `LUMEN_THEME` or the config file if the default dark theme does not suit.

## Install

```bash
cargo install --path .
```

## Workspace reviews

Run `lumiere diff` from a folder that is not a repository but has repositories
directly inside it:

```bash
cd ~/code/my-workspace      # contains api/, web/, ...
lumiere diff origin/main...-
```

- Only immediate child directories count. Symlinks and plain folders are ignored.
  Git and Jujutsu repositories are both detected.
- The first repository, by name, that has changes opens first.
- `R` opens the repository list. Each repository keeps its own threads, viewed files
  and position while Lumiere is open.
- A repository where the requested revision does not exist (for example no
  `origin/main`) stays in the list; choosing it shows the error and leaves the current
  review on screen.
- `s` sends the queued comments of every repository. Without an agent they are
  written to stdout, each group under a `# Repository: name (path)` heading.
- `--pr`, `--detect-pr` and `--stacked` need a single repository.
- The thread list (`I`), including its copy and export actions, covers the
  repository on screen only.
- With watch mode on, only the repository on screen is watched. Coming back to another
  one reloads it.

## Review threads

A comment starts a thread on a selection, a hunk or a file. The agent answers inside
the thread while Lumiere stays open.

| Key | Effect |
|---|---|
| `i` | Comment, or reply when the lines already have an open thread. An unsent comment is edited instead. |
| `Enter` | Save the comment and send it to the agent now. |
| `Alt+Enter` (Option+Enter), `Ctrl+S` | Save the comment as queued, as `Alt+Enter` queues a message in pi. Use `Ctrl+S` where the terminal does not report `Alt+Enter`. |
| `Shift+Enter`, `Ctrl+J` | New line. `Ctrl+Enter` does nothing. |
| `s` | Send every queued comment, as one message. Lumiere stays open. |
| `x` | Resolve or reopen the thread on the selection, hunk or file. Replying to a resolved thread reopens it. |
| `A` | Choose the agent, or none. |
| `I` | List the threads of the repository. |

- **Agent.** Lumiere looks for [Rex](https://rex.dev) panes running `pi` whose working
  directory is a reviewed repository or the folder Lumiere was started from, and
  attaches to the most recently active one. `--agent-block BLOCK` names the pane
  instead. The footer shows the agent and its state, or `no agent`.
- **Sending.** Lumiere pastes one message into the agent's prompt and submits it. The
  message holds each comment with its thread id, file, lines and the code it is about,
  and the reply command of the binary that runs the review, named by its full path. The
  agent can answer without looking anything up. Just before pasting, Lumiere checks
  that the pane still runs `pi`; if not, nothing is sent and the comments stay queued.
- **Without an agent** (no Rex, or `none` chosen) `Enter` queues, and `s` asks to exit
  and write the queued comments to stdout, as upstream does with annotations.
- **Quitting with queued comments** asks whether to send them, keep them queued or
  cancel.
- **Storage.** Threads are kept per repository in
  `<state dir>/lumiere/<hash of the repository root>/threads.json`
  (`~/.local/state` on Linux), outside the repository. They are shared by every diff
  range reviewed there and come back when Lumiere is restarted.
- **Outdated.** When the diff changes, a thread follows its lines if their text is
  found unchanged at exactly one place on the same side of the file. Otherwise it is
  marked `outdated`; the mark goes away if the text comes back.

The agent uses two commands, from the repository or the workspace folder:

```bash
lumiere thread list [--awaiting] [--json] [--repo PATH]
lumiere thread reply <thread-id> (--body TEXT | --stdin) [--repo PATH] [--author NAME]
```

`--awaiting` keeps the open threads whose last sent comment is the reviewer's. Queued
comments are hidden from both commands, and a resolved thread cannot be replied to.

## pi extension

`pi-extension/` adds a `/lumiere` command to the [pi](https://github.com/earendil-works/pi)
coding agent. Link it once:

```bash
ln -s "$(pwd)/pi-extension" ~/.pi/agent/extensions/lumiere
```

```
/lumiere                    review origin/main...- with watch mode on
/lumiere diff HEAD~3..-     any lumiere arguments
```

Inside [Rex](https://rex.dev) the review opens in a split pane to the right of pi,
zoomed to fill the window, and attached to that pi session. Pi keeps running behind
it: comments reach the agent and its replies appear in the review while it is open.
Unzoom the pane in Rex to see pi beside the review.

Elsewhere pi's interface is suspended while Lumiere uses the terminal. Comment, then
press `s` and Enter: the comments are sent to the agent, which is asked to answer
each one and to change code only after you approve. `q` cancels and sends nothing.

`e` opens the file in pi's `EDITOR`. The extension passes it to the review pane,
which does not inherit pi's environment; without `EDITOR`, Lumiere opens `vim`.

`LUMIERE_BIN` selects the binary when `lumiere` is not on `PATH`. The extension and
the binary must come from the same version: `/lumiere` passes `--agent-block`.

## Upstream updates

Use normal merges, so the fork's changes stay reviewable:

```bash
git fetch upstream main
git merge upstream/main
```

Resolve conflicts in fork-modified files normally. Do not rebase the fork onto
upstream and do not squash upstream history.

## Releases

Nothing is published from this fork under upstream's identity: no crates.io release
of `lumen`, no Homebrew tap update. `.github/workflows/release.yml` is upstream's
release pipeline. It only runs when started by hand and it needs secrets this fork does
not have. Do not run it.

GitHub Actions is switched off for this whole repository, because GitHub offers no
way to disable a single workflow that a fork has not registered yet. Adding CI later
means switching Actions back on:

```bash
gh api -X PUT repos/aduverger/lumiere/actions/permissions -F enabled=true
```

Before doing that, delete or rewrite `release.yml`.
