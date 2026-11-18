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
- Workspace reviews and the `/lumiere` pi command, described below.
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
- `R` opens the repository list. Each repository keeps its own annotations, viewed
  files and position while Lumiere is open.
- A repository where the requested revision does not exist (for example no
  `origin/main`) stays in the list; choosing it shows the error and leaves the current
  review on screen.
- `s` sends the annotations of every repository, each group under a
  `# Repository: name (path)` heading.
- `--pr`, `--detect-pr` and `--stacked` need a single repository.
- The annotation list (`I`), including its copy and export actions, covers the
  repository on screen only.
- With watch mode on, only the repository on screen is watched. Coming back to another
  one reloads it.

## pi extension

`pi-extension/` adds a `/lumiere` command to the [pi](https://github.com/earendil-works/pi)
coding agent. Link it once:

```bash
ln -s "$(pwd)/pi-extension" ~/.pi/agent/extensions/lumiere
```

```
/lumiere                    review origin/main...-
/lumiere diff HEAD~3..-     any lumiere arguments
```

Inside [Rex](https://rex.dev) the review opens in a split pane to the right of pi.
Elsewhere pi's interface is suspended while Lumiere uses the terminal. Annotate, then
press `s` and Enter: the annotations are sent to the agent, which is asked to answer
each comment and to change code only after you approve. `q` cancels and sends nothing.

`LUMIERE_BIN` selects the binary when `lumiere` is not on `PATH`.

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
