# <p align="center"><img src="https://github.com/user-attachments/assets/896f9239-134a-4428-9bb5-50ea59cdb5c3" alt="lumen" /></p>

[![Crates.io Total Downloads](https://img.shields.io/crates/d/lumen?label=downloads%20%40crates.io)](https://crates.io/crates/lumen)
[![GitHub Releases](https://img.shields.io/github/downloads/jnsahaj/lumen/total?label=dowloads%20%40releases)](https://github.com/jnsahaj/lumen/releases)
![GitHub License](https://img.shields.io/github/license/jnsahaj/lumen)
![Crates.io Size](https://img.shields.io/crates/size/lumen)

A command-line tool that uses AI to streamline your git workflow - generate commit messages, view diff and explain changes.
![CleanShot 2025-12-30 at 02 47 04](https://github.com/user-attachments/assets/21e9d32a-de8a-40b2-b79b-3e87c9cbb9eb)

## Table of Contents
- [Features](#features-)
- [Getting Started](#getting-started-)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
  - [Configuration (for AI features)](#configuration-for-ai-features)
- [Usage](#usage-)
  - [Visual Diff Viewer](#visual-diff-viewer)
  - [Generate Commit Messages](#generate-commit-messages)
  - [Generate Git Commands](#generate-git-commands)
  - [Explain Changes](#explain-changes)
  - [Interactive Mode](#interactive-mode)
  - [Tips & Tricks](#tips--tricks)
- [AI Providers](#ai-providers-)
- [Advanced Configuration](#advanced-configuration-)
  - [Configuration File](#configuration-file)
  - [Configuration Precedence](#configuration-precedence)

## Features 🔅

- **Beautiful & Ergonomic Diff Viewer**: Review your code with minimal effort
- **Smart Commit Messages**: Generate conventional commit messages for your staged changes
- **Git History Insights**: Understand what changed in any commit, branch, or your current work
- **Interactive Search**: Find and explore commits using fuzzy search
- **Change Analysis**: Ask questions about specific changes and their impact
- **Multiple AI Providers**: Supports OpenAI, Claude, Groq, Poe, Ollama, and more
- **Flexible**: Works with any git workflow and supports multiple AI providers
- **Rich Output**: Markdown support for readable explanations and diffs (requires: mdcat)

## Getting Started 🔅

### Prerequisites
Before you begin, ensure you have:
1. `git` installed on your system
2. [fzf](https://github.com/junegunn/fzf) (optional) - Required for `lumen explain --list` command
3. [mdcat](https://github.com/swsnr/mdcat) (optional) - Required for pretty output formatting

### Installation

#### Using Homebrew (MacOS and Linux)
```bash
brew install jnsahaj/lumen/lumen
```

#### Using Cargo
> [!IMPORTANT]
> `cargo` is a package manager for `rust`,
> and is installed automatically when you install `rust`.
> See [installation guide](https://doc.rust-lang.org/cargo/getting-started/installation.html)
```bash
cargo install lumen
```

### Configuration (for AI features)

If you want to use AI-powered features (`explain`, `draft`, `list`, `operate`), run the interactive setup:

```bash
lumen configure
```

This will guide you through:
1. Selecting an AI provider
2. Entering your API key (optional if using environment variable)
3. Specifying a custom model name (optional - press Enter to use the default)

The configuration is saved to `~/.config/lumen/lumen.config.json`.

> [!NOTE]
> The `diff` command works without any configuration - it's a standalone visual diff viewer.


## Usage 🔅

### Generate Commit Messages

Create meaningful commit messages for your staged changes:

```bash
# Basic usage - generates a commit message based on staged changes
lumen draft
# Output: "feat(button.tsx): Update button color to blue"

# Add context for more meaningful messages
lumen draft --context "match brand guidelines"
# Output: "feat(button.tsx): Update button color to align with brand identity guidelines"
```


### Generate Git Commands

Ask Lumen to generate Git commands based on a natural language query:

```bash
lumen operate "squash the last 3 commits into 1 with the message 'squashed commit'"
# Output: git reset --soft HEAD~3 && git commit -m "squashed commit" [y/N]
```

The command will display an explanation of what the generated command does, show any warnings for potentially dangerous operations, and prompt for confirmation before execution.

### Visual Diff Viewer

Launch an interactive side-by-side diff viewer in your terminal:
<img width="3456" height="2158" alt="image" src="https://github.com/user-attachments/assets/dc425871-3826-4368-88d8-931b9403f0ec" />

```bash
# View uncommitted changes
lumen diff

# View changes for a specific commit
lumen diff HEAD~1

# View changes between branches
lumen diff main..feature/A

# View changes in a GitHub Pull Request
lumen diff --pr 123 # (--pr is optional)
lumen diff https://github.com/owner/repo/pull/123

# Filter to specific files
lumen diff --file src/main.rs --file src/lib.rs

# Watch mode - auto-refresh on file changes
lumen diff --watch

# Stacked mode - review commits one by one
lumen diff main..feature --stacked
```

#### Stacked Diff Mode

Review a range of commits one at a time with `--stacked`:

```bash
lumen diff main..feature --stacked
lumen diff HEAD~5..HEAD --stacked
```

This displays each commit individually, letting you navigate through them:
- `ctrl+h` / `ctrl+l`: Previous / next commit
- Click the `‹` / `›` arrows in the header

The header shows the current commit position, SHA, and message. Viewed files are tracked per commit, so your progress is preserved when navigating.

When viewing a PR, you can mark files as viewed (syncs with GitHub) using the `space` keybinding.

#### Theme Configuration

Customize the diff viewer colors with preset themes:

```bash
# Using CLI flag
lumen diff --theme dracula

# Using environment variable
LUMEN_THEME=catppuccin-mocha lumen diff

# Or set permanently in config file (~/.config/lumen/lumen.config.json)
{
  "theme": "dracula"
}
```

**Available themes:**
| Theme | Value |
|-------|-------|
| Default (auto-detect) | `dark`, `light` |
| Catppuccin | `catppuccin-mocha`, `catppuccin-latte` |
| Dracula | `dracula` |
| Nord | `nord` |
| One Dark | `one-dark` |
| Gruvbox | `gruvbox-dark`, `gruvbox-light` |
| Solarized | `solarized-dark`, `solarized-light` |

Priority: CLI flag > config file > `LUMEN_THEME` env var > OS auto-detect.

#### Annotations

Add comments to hunks during code review:
- `i`: Add/edit annotation on focused hunk
- `I`: View all annotations (edit, delete, copy, or export)

Annotations can be copied to clipboard or exported to a file for sharing.

#### Keybindings

- `j/k` or arrow keys: Navigate
- `{/}`: Jump between hunks
- `tab`: Toggle sidebar
- `space`: Mark file as viewed
- `e`: Open file in editor
- `i/I`: Add annotation / view all annotations
- `ctrl+h/l`: Previous/next commit (stacked mode)
- `?`: Show all keybindings

### Explain Changes

Understand what changed and why:

```bash
# Explain current changes in your working directory
lumen explain                         # All changes
lumen explain --staged                # Only staged changes

# Explain specific commits
lumen explain HEAD                    # Latest commit
lumen explain abc123f                 # Specific commit
lumen explain HEAD~3..HEAD            # Last 3 commits
lumen explain main..feature/A         # Branch comparison
lumen explain main...feature/A        # Branch comparison (merge base)

# Ask specific questions about changes
lumen explain --query "What's the performance impact of these changes?"
lumen explain HEAD --query "What are the potential side effects?"

# Interactive commit selection
lumen explain --list                  # Select commit interactively
```

### Interactive Mode
```bash
# Launch interactive fuzzy finder to search through commits (requires: fzf)
lumen explain --list

# Deprecated: lumen list (use lumen explain --list instead)
```

### Tips & Tricks

```bash
# Copy commit message to clipboard
lumen draft | pbcopy                  # macOS
lumen draft | xclip -selection c      # Linux

# View the commit message and copy it
lumen draft | tee >(pbcopy)

# Open in your favorite editor
lumen draft | code -      

# Directly commit using the generated message
lumen draft | git commit -F -           
```

If you are using [lazygit](https://github.com/jesseduffield/lazygit), you can add this to the [user config](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
```yml
customCommands:
  - key: '<c-l>'
    context: 'files'
    command: 'lumen draft | tee >(pbcopy)'
    loadingText: 'Generating message...'
    showOutput: true
  - key: '<c-k>'
    context: 'files'
    command: 'lumen draft -c {{.Form.Context | quote}} | tee >(pbcopy)'
    loadingText: 'Generating message...'
    showOutput: true
    prompts:
          - type: 'input'
            title: 'Context'
            key: 'Context'
```

## AI Providers 🔅

Configure your preferred AI provider:

```bash
# Using CLI arguments
lumen -p openai -k "your-api-key" -m "gpt-5-mini" draft

# Using environment variables
export LUMEN_AI_PROVIDER="openai"
export LUMEN_API_KEY="your-api-key"
export LUMEN_AI_MODEL="gpt-5-mini"
```

### Supported Providers

| Provider | API Key Required | Models |
|----------|-----------------|---------|
| [OpenAI](https://platform.openai.com/docs/models) `openai` (Default) | Yes | `gpt-5.2`, `gpt-5`, `gpt-5-mini`, `gpt-5-nano`, `gpt-4.1`, `gpt-4.1-mini`, `o4-mini` (default: `gpt-5-mini`) |
| [Claude](https://www.anthropic.com/pricing) `claude` | Yes | `claude-sonnet-4-5-20250930`, `claude-opus-4-5-20251115`, `claude-haiku-4-5-20251015` (default: `claude-sonnet-4-5-20250930`) |
| [Gemini](https://ai.google.dev/) `gemini` | Yes (free tier) | `gemini-3-pro`, `gemini-3-flash-preview`, `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite` (default: `gemini-2.5-flash`) |
| [Groq](https://console.groq.com/docs/models) `groq` | Yes (free) | `llama-3.3-70b-versatile`, `llama-3.1-8b-instant`, `meta-llama/llama-4-maverick-17b-128e-instruct`, `openai/gpt-oss-120b` (default: `llama-3.3-70b-versatile`) |
| [DeepSeek](https://www.deepseek.com/) `deepseek` | Yes | `deepseek-chat` (V3.2), `deepseek-reasoner` (default: `deepseek-chat`) |
| [xAI](https://x.ai/) `xai` | Yes | `grok-4`, `grok-4-mini`, `grok-4-mini-fast` (default: `grok-4-mini-fast`) |
| [OpenCode Zen](https://opencode.ai/docs/zen) `opencode-zen` | Yes | [see list](https://opencode.ai/docs/zen#models) (default: `claude-sonnet-4-5`) |
| [Ollama](https://github.com/ollama/ollama) `ollama` | No (local) | [see list](https://ollama.com/library) (default: `llama3.2`) |
| [OpenRouter](https://openrouter.ai/) `openrouter` | Yes | [see list](https://openrouter.ai/models) (default: `anthropic/claude-sonnet-4.5`) |
| [Poe](https://creator.poe.com/docs/external-applications/openai-compatible-api) `poe` | Yes | [see list](https://poe.com/api/models) (default: `claude-sonnet-4.5`) |
| [Vercel AI Gateway](https://vercel.com/docs/ai-gateway) `vercel` | Yes | [see list](https://vercel.com/docs/ai-gateway/supported-models) (default: `anthropic/claude-sonnet-4.5`) |

## Advanced Configuration 🔅

### Configuration File
Lumen supports configuration through a JSON file. You can place the configuration file in one of the following locations:

1. Project Root: Create a lumen.config.json file in your project's root directory.
2. Custom Path: Specify a custom path using the --config CLI option.
3. Global Configuration (Optional): Place a lumen.config.json file in your system's default configuration directory:
    - Linux/macOS: `~/.config/lumen/lumen.config.json`
    - Windows: `%USERPROFILE%\.config\lumen\lumen.config.json`

Lumen will load configurations in the following order of priority:

1. CLI arguments (highest priority)
2. Configuration file specified by --config
3. Project root lumen.config.json
4. Global configuration file (lowest priority)

```json
{
  "provider": "openai",
  "model": "gpt-5-mini",
  "api_key": "sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "theme": "catppuccin-mocha",
  "draft": {
    "commit_types": {
      "docs": "Documentation only changes",
      "style": "Changes that do not affect the meaning of the code",
      "refactor": "A code change that neither fixes a bug nor adds a feature",
      "perf": "A code change that improves performance",
      "test": "Adding missing tests or correcting existing tests",
      "build": "Changes that affect the build system or external dependencies",
      "ci": "Changes to our CI configuration files and scripts",
      "chore": "Other changes that don't modify src or test files",
      "revert": "Reverts a previous commit",
      "feat": "A new feature",
      "fix": "A bug fix"
    }
  }
}
```

### Configuration Precedence
Options are applied in the following order (highest to lowest priority):
1. CLI Flags
2. Configuration File
3. Environment Variables
4. Default options

Example: Using different providers for different projects:
```bash
# Set global defaults in .zshrc/.bashrc
export LUMEN_AI_PROVIDER="openai"
export LUMEN_AI_MODEL="gpt-5-mini"
export LUMEN_API_KEY="sk-xxxxxxxxxxxxxxxxxxxxxxxx"

# Override per project using config file
{
  "provider": "ollama",
  "model": "llama3.2"
}

# Or override using CLI flags
lumen -p "ollama" -m "llama3.2" draft
```
## Contributors

<a href="https://github.com/jnsahaj/lumen/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=jnsahaj/lumen" />
</a>

Made with [contrib.rocks](https://contrib.rocks).

### Interested in Contributing?

Contributions are welcome! Please feel free to submit a Pull Request.

# Star History

<p align="center">
  <a target="_blank" href="https://star-history.com/#jnsahaj/lumen&Date">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=jnsahaj/lumen&type=Date&theme=dark">
      <img alt="GitHub Star History for jnsahaj/lumen" src="https://api.star-history.com/svg?repos=jnsahaj/lumen&type=Date">
    </picture>
  </a>
</p>
