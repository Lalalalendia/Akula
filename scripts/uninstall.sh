#!/usr/bin/env bash
set -euo pipefail

APP_DIR="$HOME/Applications"
APP_PATH="$APP_DIR/Aquilum.app"

if [[ -L "$APP_DIR" || ( -e "$APP_DIR" && ! -d "$APP_DIR" ) ]]; then
  printf 'Unsafe Applications directory: %s\n' "$APP_DIR" >&2
  exit 1
fi
if [[ ! -d "$APP_DIR" ]]; then
  printf 'Aquilum is not installed in %s\n' "$APP_DIR"
  printf 'Vaults, Markdown files, and Aquilum settings were left untouched.\n'
  exit 0
fi

# Use the same persistent BSD advisory lock as install/recovery. In
# particular, an uninstall must not erase an old app while an installer is
# preparing a backup or promoting the new app.
lock_file="$APP_DIR/.akula-install.lock"
if [[ -L "$lock_file" || ( -e "$lock_file" && ! -f "$lock_file" ) ]]; then
  printf 'Unsafe shared installer lock path: %s\n' "$lock_file" >&2
  exit 1
fi
exec 9>>"$lock_file"
if ! lockf -s -t 0 9; then
  printf 'Another Akula installer or uninstaller holds the Applications lock.\n' >&2
  exit 1
fi

# An interrupted installation may have the sole recoverable prior app in a
# hidden stage. Uninstalling here would obscure which generation is valid.
# Require install recovery or explicit manual review before removal.
for stale in "$APP_DIR"/.akula-install.*; do
  [[ "$stale" == "$lock_file" ]] && continue
  [[ -e "$stale" || -L "$stale" ]] || continue
  printf 'Interrupted installer state needs review before uninstall: %s\n' "$stale" >&2
  exit 1
done

if [[ -L "$APP_PATH" || ( -e "$APP_PATH" && ! -d "$APP_PATH" ) ]]; then
  printf 'Unsafe or non-directory application destination: %s\n' "$APP_PATH" >&2
  exit 1
fi
if [[ -d "$APP_PATH" ]]; then
  rm -rf "$APP_PATH"
  printf 'Removed Aquilum from %s\n' "$APP_PATH"
else
  printf 'Aquilum is not installed in %s\n' "$APP_DIR"
fi

printf 'Vaults, Markdown files, and Aquilum settings were left untouched.\n'
