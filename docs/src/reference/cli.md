# CLI Reference

```text
lumiere [GLOBAL OPTIONS] <COMMAND>
```

## Global options

Place global options **before** the subcommand (`diff` or `thread`):

| Flag | Description |
|---|---|
| `--config <PATH>` | Path to a JSON config file (overrides `~/.config/lumiere/lumiere.config.json`) |
| `--vcs <git\|jj>` | Force the version control backend instead of auto-detecting |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

---

## `lumiere diff`

Launch the interactive side-by-side diff viewer.

```text
lumiere diff [OPTIONS] [REFERENCE]
```

### Positional argument

| Argument | Description |
|---|---|
| `[REFERENCE]` | Revision target (`SHA`, `A..B`, `A...B`, `A..-`, `A...-`), or a GitHub PR number/URL. Omit for uncommitted working-tree changes. |

### Options

| Option | Description |
|---|---|
| `--pr <PR>` | View a GitHub pull request by number or URL (conflicts with `--detect-pr`) |
| `--detect-pr` | Auto-detect the GitHub PR associated with the current branch |
| `-f`, `--file <PATH>` | Filter the diff to specific file paths (repeatable) |
| `-w`, `--watch` | Watch the repository and auto-reload on file or VCS changes |
| `-t`, `--theme <NAME>` | Override color theme (see [Configuration & Themes](configuration.md)) |
| `--stacked` | Review a commit range (`A..B` or `A...B`) commit by commit |
| `--focus <PATH>` | Start with the specified file selected |
| `--origin <OWNER/REPO>` | GitHub `owner/repo` override when `--pr` is given as a number |
| `--wrap` | Soft-wrap long diff lines instead of horizontal scrolling |
| `--view <hunks\|full>` | Initial view mode (`hunks` default, `full` for whole files) |
| `--agent-block <BLOCK>` | Explicit Rex block ID of the coding agent pane to attach to |

---

## `lumiere thread`

Read and answer review threads stored by `lumiere diff`. Works from a single repository or a multi-repository workspace folder.

### `lumiere thread list`

```text
lumiere thread list [--awaiting] [--json] [--repo <PATH>]
```

| Option | Description |
|---|---|
| `--awaiting` | Show only open threads whose last sent comment is the reviewer's |
| `--json` | Emit structured JSON instead of plain text |
| `--repo <PATH>` | Repository or workspace folder path (default: current directory) |

### `lumiere thread show`

```text
lumiere thread show <THREAD_ID> [--json] [--repo <PATH>]
```

Displays the thread metadata, anchored code snippet, and all non-queued comments.

### `lumiere thread reply`

```text
lumiere thread reply <THREAD_ID> (--body <TEXT> | --stdin) [--repo <PATH>] [--author <NAME>]
```

| Option | Description |
|---|---|
| `--body <TEXT>` | Reply body text (mutually exclusive with `--stdin`) |
| `--stdin` | Read reply body from standard input |
| `--repo <PATH>` | Repository or workspace folder path (default: current directory) |
| `--author <NAME>` | Author label recorded on the reply (default: `agent`) |

Refuses to reply to unknown IDs, resolved threads, empty bodies, or threads that only contain unsent queued drafts.
