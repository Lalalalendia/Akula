#!/usr/bin/env bash
set -euo pipefail

# This script verifies the ACTUAL packaged DMG before a Release workflow
# uploads it. There is no runtime/environment-controlled publisher override.
# The trusted identity comes only from the pinned source installer in this
# exact Git checkout. Ad-hoc signed packages are not release candidates.
die() { printf 'Akula macOS release HOLD: %s\n' "$*" >&2; exit 1; }

[[ "$#" -eq 2 ]] || die 'usage: verify-macos-release.sh <source-installer> <packaged-dmg>'
installer="$1"
dmg="$2"
[[ "$(uname -s)" == Darwin ]] || die 'real macOS verification is mandatory'
[[ -f "$installer" && ! -L "$installer" ]] || die 'source installer absent or a symlink'
[[ -f "$dmg" && ! -L "$dmg" ]] || die 'DMG absent or a symlink'

# Parse exactly one unambiguous literal assignment. Never source/eval a script
# or inherit publisher identity from the release, a secret, or appdata.
read_literal() {
  local name="$1" lines value
  lines="$(grep -E "^$name=\"[^\"]*\"$" "$installer" || true)"
  [[ -n "$lines" && "$lines" != *$'\n'* ]] || die "missing or ambiguous pinned $name"
  value="${lines#*=}"
  value="${value#\"}"
  value="${value%\"}"
  printf '%s' "$value"
}
team="$(read_literal AKULA_DEVELOPER_TEAM_ID)"
bundle="$(read_literal AKULA_BUNDLE_ID)"
[[ "$team" =~ ^[A-Z0-9]{10}$ ]] || die 'Developer ID Team is not provisioned'
[[ "$bundle" =~ ^[A-Za-z0-9][A-Za-z0-9.-]+$ ]] || die 'Akula bundle identity is not provisioned'

for tool in codesign spctl hdiutil xcrun; do
  command -v "$tool" > /dev/null || die "missing macOS tool: $tool"
done

# A valid code signature is not enough: Require a stapled Apple notary ticket
# on the distributable image, in addition to pinned signer and Gatekeeper.
xcrun stapler validate "$dmg" > /dev/null ||
  die 'DMG has no valid stapled Apple notarization ticket'

mount_dir="$(mktemp -d "${TMPDIR:-/tmp}/akula-verify.XXXXXXXX")"
mounted=false
cleanup() {
  local code=$?
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$mount_dir" -quiet > /dev/null 2>&1 ||
      printf 'Akula macOS release: mount could not be detached: %s\n' "$mount_dir" >&2
  fi
  rmdir "$mount_dir" 2>/dev/null || true
  exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
hdiutil attach -nobrowse -readonly -mountpoint "$mount_dir" "$dmg" > /dev/null ||
  die 'cannot mount packaged DMG read-only'
mounted=true

app="$mount_dir/Aquilum.app"
[[ -d "$app" && ! -L "$app" ]] || die 'expected Aquilum.app absent or aliased in DMG'
[[ -f "$app/Contents/MacOS/aquilum-app" &&
   ! -L "$app/Contents/MacOS/aquilum-app" ]] ||
  die 'expected native executable missing or aliased'

requirement="=anchor apple generic and identifier \"$bundle\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$team\""
codesign --verify --deep --strict -R "$requirement" "$app" ||
  die 'packaged app is not signed by the pinned Akula Developer ID Application'
spctl --assess --type execute "$app" ||
  die 'packaged app did not pass macOS Gatekeeper assessment'

printf 'Akula macOS release: pinned publisher and stapled DMG verified\n'
