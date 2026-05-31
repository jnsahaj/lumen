#!/usr/bin/env bash
#
# Install lumen integrations into supported coding agents.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/jnsahaj/lumen/main/integrations/install.sh | bash
#   bash integrations/install.sh
#
# Currently wires up:
#   - Codex Stop hook (requires Codex's experimental hooks feature flag)
#
# Pi integration is published as an npm package; install it with:
#   pi install npm:@jnsahaj/pi-lumen-diff
#
# This script does NOT install the `lumen` binary itself — use cargo or brew
# (`cargo install lumen` / `brew install jnsahaj/lumen/lumen`) first.

set -euo pipefail

CODEX_DIR="${CODEX_HOME:-$HOME/.codex}"
CODEX_CONFIG="$CODEX_DIR/config.toml"
CODEX_HOOKS="$CODEX_DIR/hooks.json"

LUMEN_HOOK_CMD="lumen diff --hook codex-stop"

# ── Output helpers ──────────────────────────────────────────────────────

say()  { printf '%s\n' "$*"; }
ok()   { printf '  ✓ %s\n' "$*"; }
skip() { printf '  · %s\n' "$*"; }
warn() { printf '  ! %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# ── Sanity ──────────────────────────────────────────────────────────────

case "$(uname -s 2>/dev/null)" in
    MINGW*|MSYS*|CYGWIN*)
        die "Codex hooks are not supported on Windows in upstream Codex. Skipping."
        ;;
esac

if ! command -v lumen >/dev/null 2>&1; then
    warn "lumen not found on PATH. Install it first: cargo install lumen"
    warn "Continuing — the hook will fail until lumen is installed."
fi

# ── Codex install ───────────────────────────────────────────────────────

install_codex() {
    say "Codex"

    if ! command -v codex >/dev/null 2>&1 && [ ! -d "$CODEX_DIR" ]; then
        skip "no \`codex\` on PATH and $CODEX_DIR doesn't exist — skipping"
        return 0
    fi

    mkdir -p "$CODEX_DIR"

    enable_codex_feature_flag
    install_codex_hooks_json

    say ""
    say "Restart Codex to pick up the new hook."
}

enable_codex_feature_flag() {
    if [ ! -f "$CODEX_CONFIG" ]; then
        printf '[features]\nhooks = true\n' > "$CODEX_CONFIG"
        ok "created $CODEX_CONFIG with [features] hooks = true"
        return 0
    fi

    # Already enabled?
    if awk '
        BEGIN { in_features = 0 }
        /^\[/ { in_features = ($0 ~ /^\[features\][[:space:]]*$/) ; next }
        in_features && /^[[:space:]]*hooks[[:space:]]*=[[:space:]]*true/ { print "yes"; exit }
    ' "$CODEX_CONFIG" | grep -q yes; then
        skip "[features] hooks = true already set in $CODEX_CONFIG"
        return 0
    fi

    # Refuse to touch weird inline-table syntax (`features = { hooks = true }`)
    if grep -Eq '^[[:space:]]*features[[:space:]]*=' "$CODEX_CONFIG"; then
        warn "$CODEX_CONFIG uses inline \`features = ...\` syntax."
        warn "Leaving it alone. Set \`hooks = true\` inside that table manually."
        return 1
    fi

    # Insert into existing [features] block, or append a new one
    tmp=$(mktemp)
    awk '
        BEGIN { added = 0; in_features = 0 }
        /^\[features\][[:space:]]*$/ {
            print
            print "hooks = true"
            added = 1
            in_features = 1
            next
        }
        /^\[/ && in_features { in_features = 0 }
        { print }
        END {
            if (!added) {
                print ""
                print "[features]"
                print "hooks = true"
            }
        }
    ' "$CODEX_CONFIG" > "$tmp"
    mv "$tmp" "$CODEX_CONFIG"
    ok "enabled [features] hooks = true in $CODEX_CONFIG"
}

install_codex_hooks_json() {
    local payload
    payload=$(cat <<EOF
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "$LUMEN_HOOK_CMD",
            "timeout": 345600
          }
        ]
      }
    ]
  }
}
EOF
)

    if [ ! -f "$CODEX_HOOKS" ]; then
        printf '%s\n' "$payload" > "$CODEX_HOOKS"
        ok "wrote $CODEX_HOOKS"
        return 0
    fi

    if grep -Fq "$LUMEN_HOOK_CMD" "$CODEX_HOOKS"; then
        skip "$CODEX_HOOKS already references the lumen hook"
        return 0
    fi

    warn "$CODEX_HOOKS already exists with other hooks."
    warn "Add this Stop hook manually:"
    printf '\n%s\n\n' "$payload" >&2
    return 1
}

# ── Run ─────────────────────────────────────────────────────────────────

install_codex
