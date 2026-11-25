# Configuration & Themes

## Configuration file

Lumiere reads `~/.config/lumiere/lumiere.config.json` by default, or the path supplied with `--config <PATH>` before `diff`.

```json
{
  "theme": "catppuccin-mocha",
  "wrap": false,
  "view": "hunks"
}
```

### Supported keys

| Key | Type | Default | Description |
|---|---|---|---|
| `theme` | `string` | auto-detected | Color theme name (see table below) |
| `wrap` | `boolean` | `false` | Soft-wrap long diff lines instead of horizontal scrolling |
| `view` | `"hunks" \| "full"` | `"hunks"` | Default diff view mode (`z` toggles interactively) |

A missing default config file is silently ignored. A malformed JSON file or a missing explicit `--config` path is reported as a clear error on startup.

## Precedence rules

- **Theme:** `--theme` CLI flag → `theme` in config file → `LUMIERE_THEME` environment variable → terminal light/dark background auto-detection (`terminal-light`).
- **Wrap:** `--wrap` CLI flag OR `wrap: true` in config file.
- **View mode:** `--view` CLI flag → `view` in config file → `"hunks"`.

## Built-in themes

Theme names are matched case-insensitively (hyphens and underscores are interchangeable for most variants):

| Family | Theme name(s) | Mode |
|---|---|---|
| Default | `dark`, `light` | Dark / Light |
| Catppuccin | `catppuccin-mocha` (alias `catppuccin`), `catppuccin-latte` | Dark / Light |
| Dracula | `dracula` | Dark |
| Nord | `nord` | Dark |
| One Dark | `one-dark` (alias `onedark`) | Dark |
| Gruvbox | `gruvbox-dark` (alias `gruvbox`), `gruvbox-light` | Dark / Light |
| Solarized | `solarized-dark` (alias `solarized`), `solarized-light` | Dark / Light |
| Flexoki | `flexoki-dark` (alias `flexoki`), `flexoki-light` | Dark / Light |

## Environment variables

| Variable | Effect |
|---|---|
| `LUMIERE_THEME` | Default theme when neither `--theme` nor `config.theme` is set |
| `EDITOR` | External editor launched by <kbd>e</kbd> (supports arguments, e.g. `code --wait`; defaults to `vim`) |
| `XDG_STATE_HOME` | Base directory for persisted `threads.json` files on Linux (defaults to `~/.local/state`) |
| `REX_SESSION` | Current Rex session ID used with `--agent-block` |
| `LUMIERE_BIN` | Override binary path used by the `pi-extension/` `/lumiere` command |
