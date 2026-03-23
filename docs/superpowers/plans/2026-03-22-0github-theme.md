# 0github Theme Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 0github-inspired diff/syntax highlighting to the `lumen` fork in a way that is testable, selectable, and consistent with the reference palette in `manaflow`.

**Architecture:** Extend `lumen`'s existing `ThemePreset` system rather than changing the diff renderer. The theme layer already owns syntax colors, diff backgrounds, CLI parsing, and environment/config selection, so a 0github preset can be introduced with narrow changes and verified through unit tests.

**Tech Stack:** Rust, ratatui, clap, tree-sitter, cargo test

---

### Task 1: Add failing theme tests

**Files:**
- Modify: `src/command/diff/theme.rs`
- Reference: `/Users/lawrence/fun/manaflow/apps/www/app/globals.css`

- [ ] **Step 1: Write the failing test**

Add tests that assert:
- `ThemePreset::from_str("0github-light")` and `ThemePreset::from_str("0github-dark")` parse successfully.
- The resulting theme syntax and diff colors match the 0github palette copied from `manaflow`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test theme::tests::test_0github_theme_parsing theme::tests::test_0github_light_palette theme::tests::test_0github_dark_palette`
Expected: FAIL because the new preset does not exist yet.

### Task 2: Implement 0github presets

**Files:**
- Modify: `src/command/diff/theme.rs`

- [ ] **Step 1: Add minimal implementation**

Add:
- `ThemePreset::ZeroGithubLight`
- `ThemePreset::ZeroGithubDark`
- `FromStr` aliases for `0github-light`, `0github-dark`, and a reasonable shorthand
- `Theme::zero_github_light()` and `Theme::zero_github_dark()`
- `Theme::from_preset(...)` match arms

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test theme::tests::test_0github_theme_parsing theme::tests::test_0github_light_palette theme::tests::test_0github_dark_palette`
Expected: PASS

### Task 3: Document the preset

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update user-facing docs**

Add the new theme names to the diff theme examples and available theme table.

- [ ] **Step 2: Run lightweight verification**

Run: `cargo test theme::tests::test_0github_theme_parsing theme::tests::test_0github_light_palette theme::tests::test_0github_dark_palette`
Expected: PASS
