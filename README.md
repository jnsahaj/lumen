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
    },
    "system_prompt": "You are a commit message generator that creates semantic commit messages following conventional commits specification.",
    "user_prompt": "Generate a semantic commit message for the following changes. Use conventional commits format and be descriptive:"
  },
  "explain": {
    "system_prompt": "You are a helpful Git assistant that provides detailed explanations of changes. Always be thorough and educational.",
    "user_prompt": "Please analyze the following changes and provide a comprehensive explanation including technical details and potential impacts:"
  },
  "operate": {
    "system_prompt": "You are a Git expert that provides safe and educational Git commands with detailed explanations.",
    "user_prompt": "For the query: {query}\n\nProvide:\n- The exact Git command\n- Step-by-step explanation\n- Any safety considerations\n\nFormat:\n<command>git command here</command>\n<explanation>detailed explanation</explanation>\n<warning>safety notes if needed</warning>"
  }
}
```

### Prompt Customization

Lumen allows you to customize the AI prompts for all three main commands (`explain`, `draft`, and `operate`) to better suit your workflow and requirements. Each prompt configuration accepts:

- **`system_prompt`**: Defines the AI's role and behavior
- **`user_prompt`**: Defines the template for user input to the AI

#### Key Features

- **Fallback Support**: If a custom prompt is empty or not provided, Lumen automatically uses the built-in default prompts
- **Template Variables**: Use placeholders like `{query}`, `{diff}`, etc., that get replaced with actual content
- **Full Customization**: Completely override the default behavior to match your team's standards

#### Example Use Cases

**Code Review Focus:**

```json
{
  "explain": {
    "system_prompt": "You are a senior code reviewer. Focus on security, performance, and maintainability concerns.",
    "user_prompt": "Review these changes for potential issues:\n\n{base_content}\n\nHighlight: security risks, performance impacts, and maintainability concerns."
  }
}
```

**Educational Context:**

```json
{
  "explain": {
    "system_prompt": "You are a computer science teacher. Explain git changes in an educational manner suitable for beginners.",
    "user_prompt": "Explain these changes to a beginner programmer, including relevant programming concepts:\n\n{base_content}"
  }
}
```

**Team-Specific Commit Standards:**

```json
{
  "draft": {
    "commit_types": {
      "feat": "A new feature",
      "fix": "A bug fix",
      "docs": "Documentation only changes",
      "compliance": "Regulatory compliance changes"
    },
    "system_prompt": "You are a commit message generator for a fintech company. Follow strict regulatory compliance standards.",
    "user_prompt": "Generate a commit message that includes compliance tracking. Changes:\n\n{diff}\n\nEnsure the message follows our SOX compliance requirements."
  }
}
```

**Structured Commit Messages with Bullet Points:**

```json
{
  "draft": {
    "commit_types": {
      "feat": "A new feature",
      "fix": "A bug fix",
      "docs": "Documentation only changes",
      "style": "Changes that do not affect the meaning of the code",
      "refactor": "A code change that neither fixes a bug nor adds a feature",
      "perf": "A code change that improves performance",
      "test": "Adding missing tests or correcting existing tests"
    },
    "system_prompt": "You are a commit message generator that follows these rules:\n1. IMPORTANT: Each line must be no more than 65 characters (including title and bullet points)\n2. Write in present tense\n3. Be concise and direct\n4. Output only the commit message without any explanations\n5. Follow the format:\n   <type>(<optional scope>): <commit message>\n   \n   - <bullet point describing a key change>\n   - <bullet point describing another key change>",
    "user_prompt": "Generate a structured git commit message written in present tense for the following code diff with the given specifications below:\nThe output response must be in format:\n<type>(<optional scope>): <commit message>\n- <bullet point describing a key change>\n- <bullet point describing another key change>\nChoose a type from the type-to-description JSON below that best describes the git diff:\n{commit_types}\nFocus on being accurate and concise.\nFirst line must be a maximum of 72 characters.\nEach bullet point must be a maximum of 78 characters.\nIf a bullet point exceeds 78 characters, wrap it with 2 spaces at the start of the next line.\nExclude anything unnecessary such as translation. Your entire response will be passed directly into git commit.\nCode diff:\n```diff\n{diff}\n```"
  }
}
```

**Advanced Git Operations:**

```json
{
  "operate": {
    "system_prompt": "You are a Git expert for a large enterprise team. Always prioritize safety and provide comprehensive explanations.",
    "user_prompt": "Enterprise Git Operation Request: {query}\n\nProvide:\n1. Safe command with full explanation\n2. Potential risks and mitigation\n3. Alternative approaches\n4. Team notification requirements\n\nFormat:\n<command>git command</command>\n<explanation>enterprise-level explanation</explanation>\n<warning>comprehensive safety analysis</warning>"
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
