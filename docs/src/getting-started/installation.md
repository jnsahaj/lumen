# Installation

Lumiere targets **Linux** and **macOS** and requires **Rust 1.89 or later** when building from source.

## Install with Cargo

Install the latest version directly from GitHub:

```bash
cargo install --git https://github.com/aduverger/lumiere --locked
```

Or from a local clone:

```bash
git clone https://github.com/aduverger/lumiere.git
cd lumiere
cargo install --path . --locked
```

Both commands place the `lumiere` binary in `~/.cargo/bin`.

### Building without Jujutsu (`jj-lib`)

Jujutsu support is enabled by default via the `jj` Cargo feature. If you only use Git and want a smaller binary and faster compile time, build with `--no-default-features`:

```bash
cargo install --path . --locked --no-default-features
```

## Pre-built binaries

Tagged releases publish pre-built archives and SHA-256 checksums on the [GitHub Releases](https://github.com/aduverger/lumiere/releases) page for:

- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin` (Apple Silicon)
- `x86_64-apple-darwin` (Intel macOS)

## Nix Flake

If you use Nix with flakes enabled:

```bash
nix run github:aduverger/lumiere -- diff
```

Or install it into your profile:

```bash
nix profile install github:aduverger/lumiere
```

## Optional integrations

| Tool | Needed for |
|---|---|
| [GitHub CLI (`gh`)](https://cli.github.com/) | Reviewing GitHub PRs via `--pr` / `--detect-pr` and syncing file viewed checkboxes |
| [Rex](https://rex.dev) | Live two-way review threads with coding agents running in sibling terminal panes |
| [pi](https://github.com/earendil-works/pi) | The `/lumiere` slash command inside pi (see below) |

## Installing the `pi` extension

If you use the [pi](https://github.com/earendil-works/pi) coding agent, symlink `pi-extension/` once from your checkout and run `/reload` inside pi:

```bash
ln -s "$(pwd)/pi-extension" ~/.pi/agent/extensions/lumiere
```

See [Coding Agents & Rex Integration](../guide/coding-agents.md) for how `/lumiere` behaves inside and outside Rex.
