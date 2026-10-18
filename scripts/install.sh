#!/usr/bin/env bash
set -euo pipefail

REPO="Lalalalendia/Akula"
APP_DIR="$HOME/Applications"
APP_PATH="$APP_DIR/Aquilum.app"

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf 'Aquilum curl installer currently supports macOS only.\n' >&2
  exit 1
fi

case "$(uname -m)" in
  arm64) target="aarch64" ;;
  x86_64) target="x64" ;;
  *) printf 'Aquilum DMG installer supports Apple Silicon and Intel Macs only.\n' >&2; exit 1 ;;
esac

work_dir="$(mktemp -d)"
mount_dir="$work_dir/mount"
mkdir "$mount_dir"
mounted=false
stage_dir=""
backup_app=""
rollback_pending=false
keep_stage=false
cleanup() {
  local status=$?
  # Restore the previous app if the new one could not be promoted. The
  # backup is on the same filesystem as Applications, so this is a rename,
  # never a partial recursive copy over an existing app.
  if [[ "$rollback_pending" == true && -n "$backup_app" && -e "$backup_app" ]]; then
    if [[ ! -e "$APP_PATH" && ! -L "$APP_PATH" ]]; then
      if ! mv "$backup_app" "$APP_PATH"; then
        printf 'Recovery failed: previous application is preserved at %s\n' "$backup_app" >&2
        keep_stage=true
        status=1
      fi
    else
      printf 'Recovery refused: destination unexpectedly occupied; old app at %s\n' "$backup_app" >&2
      keep_stage=true
      status=1
    fi
  fi
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$mount_dir" -quiet || true
  fi
  if [[ -n "$stage_dir" && "$keep_stage" != true ]]; then
    rm -rf "$stage_dir"
  fi
  rm -rf "$work_dir"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Akula has no published release channel yet. Never fall back to the
# upstream Aquilum release if this fork's latest release is unavailable.
if ! curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" -o "$work_dir/release.json"; then
  printf 'No Akula release available from %s; existing application and vaults untouched.\n' "$REPO" >&2
  exit 1
fi
download_url="$(sed -n 's/^[[:space:]]*"browser_download_url":[[:space:]]*"\(.*\)",\{0,1\}$/\1/p' "$work_dir/release.json" | grep "_${target}\.dmg$" | head -n 1 || true)"
if [[ -z "$download_url" ]]; then
  printf 'Latest GitHub release has no macOS %s DMG.\n' "$target" >&2
  exit 1
fi
# browser_download_url must remain scoped to this fork. HTTPS redirects from
# GitHub to its asset CDN are handled by curl after selecting this exact URL.
case "$download_url" in
  "https://github.com/$REPO/releases/download/"*) ;;
  *) printf 'Release metadata selected an unexpected origin; refusing install.\n' >&2; exit 1 ;;
esac

curl -fL --proto '=https' --tlsv1.2 "$download_url" -o "$work_dir/Aquilum.dmg"
hdiutil attach -nobrowse -readonly -mountpoint "$mount_dir" "$work_dir/Aquilum.dmg" >/dev/null
mounted=true
source_app="$(find "$mount_dir" -maxdepth 3 -type d -name 'Aquilum.app' -print -quit)"
if [[ -z "$source_app" ]]; then
  printf 'Aquilum.app was not found in the downloaded DMG.\n' >&2
  exit 1
fi

# Stage and inspect the new bundle BEFORE moving or replacing the old
# installation. This also keeps the candidate on the same filesystem as the
# final Applications directory, so promotion can use a single rename.
mkdir -p "$APP_DIR"
if [[ -L "$APP_PATH" || ( -e "$APP_PATH" && ! -d "$APP_PATH" ) ]]; then
  printf 'Unsafe or non-directory application destination: %s\n' "$APP_PATH" >&2
  exit 1
fi
stage_dir="$(mktemp -d "$APP_DIR/.akula-install.XXXXXXXX")"
staged_app="$stage_dir/Aquilum.app"
backup_app="$stage_dir/Aquilum.previous.app"
ditto "$source_app" "$staged_app"
if [[ ! -f "$staged_app/Contents/MacOS/aquilum-app" ]]; then
  printf 'Downloaded bundle has no expected native binary; keeping old application.\n' >&2
  exit 1
fi
# Integrity validation only: an ad-hoc code signature does not authenticate
# the publisher. Independent Akula signing identity is a separate release gate.
codesign --verify --deep --strict "$staged_app"

if [[ -e "$APP_PATH" ]]; then
  mv "$APP_PATH" "$backup_app"
  rollback_pending=true
fi
if ! mv "$staged_app" "$APP_PATH"; then
  printf 'New application promotion failed; restoring previous installation.\n' >&2
  exit 1
fi
rollback_pending=false
printf 'Installed Aquilum to %s\n' "$APP_PATH"
