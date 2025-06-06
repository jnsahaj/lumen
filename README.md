# <p align="center"><img src="https://github.com/user-attachments/assets/896f9239-134a-4428-9bb5-50ea59cdb5c3" alt="lumen" /></p>

[![Crates.io Total Downloads](https://img.shields.io/crates/d/lumen?label=downloads%20%40crates.io)](https://crates.io/crates/lumen)
[![GitHub Releases](https://img.shields.io/github/downloads/jnsahaj/lumen/total?label=dowloads%20%40releases)](https://github.com/jnsahaj/lumen/releases)
![GitHub License](https://img.shields.io/github/license/jnsahaj/lumen)
![Crates.io Size](https://img.shields.io/crates/size/lumen)

A command-line tool that uses AI to streamline your git workflow - from generating commit messages to explaining complex changes, all without requiring an API key.

![demo](https://github.com/user-attachments/assets/0d029bdb-3b11-4b5c-bed6-f5a91d8529f2)

## GitAds Sponsored
[![Sponsored by GitAds](https://gitads.dev/v1/ad-serve?source=jnsahaj/lumen@github)](https://gitads.dev/v1/ad-track?source=jnsahaj/lumen@github)

## Table of Contents
- [Features](#features-)
- [Getting Started](#getting-started-)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
- [Usage](#usage-)
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

- **Smart Commit Messages**: Generate conventional commit messages for your staged changes
- **Git History Insights**: Understand what changed in any commit, branch, or your current work
- **Interactive Search**: Find and explore commits using fuzzy search
- **Change Analysis**: Ask questions about specific changes and their impact
- **Zero Config**: Works instantly without an API key, using Phind by default
- **Flexible**: Works with any git workflow and supports multiple AI providers
- **Rich Output**: Markdown support for readable explanations and diffs (requires: mdcat)

## Getting Started 🔅

### Prerequisites
Before you begin, ensure you have:
1. `git` installed on your system
2. [fzf](https://github.com/junegunn/fzf) (optional) - Required for `lumen list` command
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

### Explain Changes

Understand what changed and why:

```bash
# Explain current changes in your working directory
lumen explain --diff                  # All changes
lumen explain --diff --staged         # Only staged changes

# Explain specific commits
lumen explain HEAD                    # Latest commit
lumen explain abc123f                 # Specific commit
lumen explain HEAD~3..HEAD            # Last 3 commits
lumen explain main..feature/A         # Branch comparison
lumen explain main...feature/A        # Branch comparison (merge base)

# Ask specific questions about changes
lumen explain --diff --query "What's the performance impact of these changes?"
lumen explain HEAD --query "What are the potential side effects?"
```

### Interactive Mode
```bash
# Launch interactive fuzzy finder to search through commits (requires: fzf)
lumen list
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
lumen -p openai -k "your-api-key" -m "gpt-4o" draft

# Using environment variables
export LUMEN_AI_PROVIDER="openai"
export LUMEN_API_KEY="your-api-key"
export LUMEN_AI_MODEL="gpt-4o"
```

### Supported Providers

| Provider | API Key Required | Models |
|----------|-----------------|---------|
| [Phind](https://www.phind.com/agent) `phind` (Default) | No | `Phind-70B` |
| [Groq](https://groq.com/) `groq` | Yes (free) | `llama2-70b-4096`, `mixtral-8x7b-32768` (default: `mixtral-8x7b-32768`) |
| [OpenAI](https://platform.openai.com/docs/guides/text-generation/chat-completions-api) `openai` | Yes | `gpt-4o`, `gpt-4o-mini`, `gpt-4`, `gpt-3.5-turbo` (default: `gpt-4o-mini`) |
| [Claude](https://claude.ai/new) `claude` | Yes | [see list](https://docs.anthropic.com/en/docs/about-claude/models#model-names) (default: `claude-3-5-sonnet-20241022`) |
| [Ollama](https://github.com/ollama/ollama) `ollama` | No (local) | [see list](https://github.com/ollama/ollama/blob/main/docs/api.md#model-names) (required) |
| [OpenRouter](https://openrouter.ai/) `openrouter` | Yes | [see list](https://openrouter.ai/models) (default: `anthropic/claude-3.5-sonnet`) |
| [DeepSeek](https://www.deepseek.com/) `deepseek` | Yes | `deepseek-chat`, `deepseek-reasoner` (default: `deepseek-reasoner`) |

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
  "model": "gpt-4o",
  "api_key": "sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
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
export LUMEN_AI_MODEL="gpt-4o"
export LUMEN_API_KEY="sk-xxxxxxxxxxxxxxxxxxxxxxxx"

# Override per project using config file
{
  "provider": "ollama",
  "model": "llama3.2"
}

# Or override using CLI flags
lumen -p "ollama" -m "llama3.2" draft
```
<!-- GitAds-Verify: OE99H8YHI6ACIS31OJLLV19T6QOB4J3P -->
