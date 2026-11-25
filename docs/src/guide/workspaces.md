# Multi-Repository Workspaces

When a feature spans multiple repositories — such as an `api/` backend and a `web/` frontend checked out under a shared folder — you can review them all in one Lumiere session.

## Starting a workspace review

Run `lumiere diff` from a parent folder that is **not** itself a Git or Jujutsu repository, but contains repositories as immediate subdirectories:

```bash
cd ~/code/my-project     # contains api/, web/, shared/
lumiere diff origin/main...- --watch
```

### Discovery rules

- Only **immediate child directories** containing `.git` or `.jj` are discovered, sorted alphabetically by directory name.
- Plain folders without a VCS marker and symlinked directories are ignored.
- Lumiere opens the first repository (by name) that has changes for the requested reference, or the first reviewable repository if all are clean.
- If a repository lacks the requested revision (for example, `origin/main` does not exist in one repo), it remains visible in the repository picker and displays the error if selected.

## Switching repositories and managing state

- Press <kbd>R</kbd> to open the **Repositories** picker. It marks the current repository, shows thread counts for every opened repository, and flags unavailable repositories.
- The footer prefixes the current branch or reference with the repository name (`api · origin/main...-`).
- Switching repositories parks the previous repository's full `AppState` in memory — preserving its open threads, viewed files, viewed hunks, collapsed directories, and scroll position.
- Watch mode (<kbd>w</kbd>) follows the repository currently on screen; returning to a parked repository while watch mode is enabled triggers a fresh reload so edits made while off-screen are picked up.

## Threads and agents across a workspace

- **Per-repository storage:** Each repository keeps its own `threads.json` under Lumiere's state directory.
- **Combined sending:** Pressing <kbd>s</kbd> sends (or exports) queued comments across **every** repository opened in the session, grouped under `# Repository: <name> (<absolute-path>)` headings.
- **Agent discovery:** Lumiere searches Rex for agents working in the workspace folder or any discovered repository root (or subdirectories thereof).
- **Agent CLI:** `lumiere thread list`, `show`, and `reply` work seamlessly from the workspace folder (`--repo ~/code/my-project`) — locating threads by ID across all child repositories and showing each repository's name in listings.

## Workspace limitations

- `--pr`, `--detect-pr`, and `--stacked` require a single repository and report an error if run from a multi-repository workspace folder.
- The threads list (<kbd>I</kbd>) shows threads for the repository currently on screen.
