#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary_home="$(mktemp -d)"
trap 'rm -rf "$temporary_home"' EXIT

mkdir -p "$temporary_home/Applications/Aquilum.app" "$temporary_home/Notes"
printf 'keep\n' > "$temporary_home/Notes/Keep.md"
HOME="$temporary_home" bash "$script_dir/uninstall.sh" >/dev/null

[[ ! -e "$temporary_home/Applications/Aquilum.app" ]]
[[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]

# The fork has no Akula release yet: a failed fetch must not silently install
# upstream Aquilum or delete the already installed app and user vault.
mkdir -p "$temporary_home/Applications/Aquilum.app" "$temporary_home/tools"
printf 'installed app remains\n' > "$temporary_home/Applications/Aquilum.app/Keep.txt"
cat > "$temporary_home/tools/uname" <<'EOF'
#!/bin/sh
case "$1" in
  -s) printf 'Darwin\n' ;;
  -m) printf 'arm64\n' ;;
  *) exit 1 ;;
esac
EOF
cat > "$temporary_home/tools/curl" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$AKULA_INSTALL_CURL_CALLS"
exit 22
EOF
chmod +x "$temporary_home/tools/uname" "$temporary_home/tools/curl"

calls="$temporary_home/curl-requests.log"
error_log="$temporary_home/install-error.log"
if HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" AKULA_INSTALL_CURL_CALLS="$calls" \
    bash "$script_dir/install.sh" > /dev/null 2> "$error_log"; then
  echo 'Installer unexpectedly succeeded without a fork release' >&2
  exit 1
fi
grep -Fq 'https://api.github.com/repos/Lalalalendia/Akula/releases/latest' "$calls"
if grep -Fq 'Freaction/Aquilum' "$calls"; then
  echo 'Installer requested the upstream release despite being in Akula' >&2
  exit 1
fi
grep -Fq 'No Akula release available' "$error_log"
[[ "$(cat "$temporary_home/Applications/Aquilum.app/Keep.txt")" == 'installed app remains' ]]
[[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]


# Offline macOS installer acceptance. Mock only external OS utilities; execute
# the ACTUAL installer with a real filesystem and a preserved existing bundle.
cat > "$temporary_home/tools/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$AKULA_INSTALL_CURL_CALLS"
dest="${@: -1}"
if [[ "$*" == *"/releases/latest"* ]]; then
  cat > "$dest" <<JSON
{
  "assets": [
    {
      "browser_download_url": "https://github.com/${AKULA_TEST_ASSET_REPO:-Lalalalendia/Akula}/releases/download/v-test/Aquilum_0.3.9_aarch64.dmg"
    }
  ]
}
JSON
else
  printf 'offline fake DMG\n' > "$dest"
fi
EOF
cat > "$temporary_home/tools/hdiutil" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  attach)
    prev=''
    mount=''
    for arg in "$@"; do
      if [[ "$prev" == '-mountpoint' ]]; then mount="$arg"; fi
      prev="$arg"
    done
    [[ -n "$mount" ]]
    mkdir -p "$mount/Aquilum.app/Contents/MacOS"
    if [[ "${AKULA_TEST_MISSING_BINARY:-0}" != 1 ]]; then
      printf 'new installed version\n' > "$mount/Aquilum.app/Contents/MacOS/aquilum-app"
    fi
    ;;
  detach) ;;
  *) exit 1 ;;
esac
EOF
cat > "$temporary_home/tools/ditto" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${AKULA_TEST_DITTO_FAIL:-0}" == 1 ]]; then
  mkdir -p "$2"
  printf 'incomplete\n' > "$2/incomplete"
  exit 27
fi
cp -R "$1" "$2"
EOF
cat > "$temporary_home/tools/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${AKULA_TEST_CODESIGN_FAIL:-0}" == 1 ]]; then exit 28; fi
[[ "$1" == --verify && "$2" == --deep && "$3" == --strict ]]
[[ -f "$4/Contents/MacOS/aquilum-app" ]]
EOF
cat > "$temporary_home/tools/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${AKULA_TEST_PROMOTION_FAIL:-0}" == 1
   && "$1" == *"/.akula-install."*"/Aquilum.app"
   && "$2" == "$HOME/Applications/Aquilum.app" ]]; then
  exit 29
fi
exec /bin/mv "$@"
EOF
chmod +x "$temporary_home/tools/"*

existing="$temporary_home/Applications/Aquilum.app"
mkdir -p "$existing/Contents/MacOS"
printf 'old installed version\n' > "$existing/Contents/MacOS/aquilum-app"

run_installer() {
  HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" \
    AKULA_INSTALL_CURL_CALLS="$calls" \
    bash "$script_dir/install.sh" > "$temporary_home/install-stdout.log" 2> "$error_log"
}
old_intact() {
  [[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'old installed version' ]]
  [[ "$(cat "$existing/Keep.txt")" == 'installed app remains' ]]
  [[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]
}
no_staging_left() {
  [[ -z "$(find "$temporary_home/Applications" -maxdepth 1 -type d -name '.akula-install.*' -print -quit)" ]]
}
must_fail_without_changing_old() {
  if run_installer; then
    echo 'Unsafe macOS installer unexpectedly succeeded' >&2
    exit 1
  fi
  old_intact
  no_staging_left
}

export AKULA_TEST_ASSET_REPO='Freaction/Aquilum'
must_fail_without_changing_old
grep -Fq 'unexpected origin' "$error_log"
unset AKULA_TEST_ASSET_REPO

export AKULA_TEST_DITTO_FAIL=1
must_fail_without_changing_old
unset AKULA_TEST_DITTO_FAIL

export AKULA_TEST_MISSING_BINARY=1
must_fail_without_changing_old
unset AKULA_TEST_MISSING_BINARY

export AKULA_TEST_CODESIGN_FAIL=1
must_fail_without_changing_old
unset AKULA_TEST_CODESIGN_FAIL

export AKULA_TEST_PROMOTION_FAIL=1
must_fail_without_changing_old
grep -Fq 'restoring previous installation' "$error_log"
unset AKULA_TEST_PROMOTION_FAIL

run_installer
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'new installed version' ]]
[[ ! -e "$existing/Keep.txt" ]]
[[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]
no_staging_left
