# Review Targets & References

`lumiere diff` accepts a wide set of Git, Jujutsu, and GitHub pull request targets. You can invoke it from any subdirectory of a repository.

## Revision reference syntax

| Reference | Meaning | Typical use |
|---|---|---|
| *(none)* | Working tree vs parent (`HEAD` in Git, `@-` in Jujutsu) | Review uncommitted changes |
| `SHA` / `REF` | Single commit vs its parent | Inspect one commit |
| `A..B` | Tree diff from `A` to `B` | Compare two commits or branches directly |
| `A...B` | Tree diff from `merge-base(A, B)` to `B` | Compare a branch tip against where it forked |
| `A..-` | Tree diff from `A` to the current working tree | Compare everything from `A` including uncommitted edits |
| `A...-` | Tree diff from `merge-base(A, HEAD)` to the working tree | **Best for feature branches** — ignores commits added to `A` after you branched |

> **Why `origin/main...-`?**
> When `origin/main` advances while you work on a feature branch, `origin/main..-` starts showing changes other teammates merged into `main`. Using three dots (`origin/main...-`) computes the merge base with your current commit and compares it to your live working tree, so you always review only your branch plus uncommitted changes.

## Watch mode (`--watch` / `-w`)

Pass `--watch` (or toggle it interactively with <kbd>w</kbd>) to reload automatically when files change on disk or when Git writes refs/index updates during a commit or checkout:

```bash
lumiere diff origin/main...- --watch
```

- Changes are gathered over a 500 ms debounce window so rapid saves or formatter passes collapse into a single reload.
- Files that change on disk are automatically unmarked as viewed so you never miss new edits by an agent or formatter.
- On **Linux**, the watcher walks the repository and skips directories ignored by Git (such as `node_modules/`, `.venv/`, and `target/`), avoiding `inotify` watch exhaustion and startup delays.
- Watch mode is unavailable in GitHub PR mode (`--pr` / `--detect-pr`).

## Stacked commit review (`--stacked`)

Review a multi-commit branch one commit at a time:

```bash
lumiere diff main..feature --stacked
lumiere diff main...feature --stacked
```

- A compact header bar displays the VCS (`git` or `jj`), commit index (`1/6`), commit ID (plus short `change_id` in Jujutsu), and commit summary.
- Press <kbd>Ctrl+L</kbd> / <kbd>Ctrl+H</kbd> (or click `‹` / `›` in the header) to step forward and backward through commits.
- Empty commits are automatically filtered out.
- Viewed-file checkboxes are remembered independently per commit SHA while switching back and forth.

## GitHub pull requests (`--pr` / `--detect-pr`)

Review a GitHub PR without checking out its branch locally (requires an authenticated [`gh`](https://cli.github.com/) CLI):

```bash
# By PR number (uses origin remote, or override with --origin owner/repo)
lumiere diff --pr 123
lumiere diff 123

# By full URL
lumiere diff --pr https://github.com/owner/repo/pull/123
lumiere diff https://github.com/owner/repo/pull/123

# Auto-detect the PR for the current branch
lumiere diff --detect-pr
```

- Base and head file contents are fetched in parallel (bounded to 8 concurrent `gh api` workers to stay comfortably below GitHub secondary rate limits).
- Fork PRs (including PRs whose fork repository has since been deleted) are supported cleanly.
- File viewed status (`Space`) syncs bidirectionally with GitHub's "Viewed" checkboxes via GraphQL mutations.
- Press <kbd>o</kbd> to open the current file directly in your browser at its exact `#diff-<sha256>` anchor on GitHub.

## Filtering and focusing files

```bash
# Limit the review to specific paths
lumiere diff -f src/lib.rs -f src/ui/mod.rs

# Open with a specific file selected first
lumiere diff --focus src/ui/mod.rs
```

## Git and Jujutsu (`--vcs`)

Lumiere automatically detects whether the directory is a Jujutsu repository (`.jj`), a colocated Jujutsu+Git repository (prefers Jujutsu by default), or a Git repository (`.git`). Override auto-detection with the global `--vcs` flag before `diff`:

```bash
lumiere --vcs git diff
lumiere --vcs jj diff @---..@
```

In Jujutsu repositories, Lumiere also prints helpful translation hints if Git-style refs such as `HEAD~2` are accidentally supplied (`@---`).
