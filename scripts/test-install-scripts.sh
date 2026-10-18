#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temporary_home="$(mktemp -d)"
trap 'rm -rf "$temporary_home"' EXIT

# macOS tests exercise the real BSD lockf utility. Ubuntu CI adapts
# its fd-lock interface to util-linux flock without changing install code.
mkdir -p "$temporary_home/tools"
if [[ "$(uname -s)" != Darwin ]]; then
  cat > "$temporary_home/tools/lockf" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -eq 4 && "$1" == -s && "$2" == -t && "$3" == 0 ]] || exit 60
[[ "$4" =~ ^[0-9]+$ ]] || exit 60
exec /usr/bin/flock -n "$4"
EOF
  chmod +x "$temporary_home/tools/lockf"
fi

mkdir -p "$temporary_home/Applications/Aquilum.app" "$temporary_home/Notes"
printf 'keep\n' > "$temporary_home/Notes/Keep.md"
HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" bash "$script_dir/uninstall.sh" >/dev/null

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

# PRODUCTION INSTALLER: signing identity is deliberately absent until an
# Akula Developer ID and bundle identifier have been independently enrolled.
# It must fail without a single API/download request, before replacing any
# existing app. Recovery itself remains a separate earlier operation.
if HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" AKULA_INSTALL_CURL_CALLS="$calls" \
    bash "$script_dir/install.sh" > /dev/null 2> "$error_log"; then
  echo 'Production installer accepted a release without publisher identity' >&2
  exit 1
fi
grep -Fq 'publisher identity is not provisioned' "$error_log"
[[ ! -e "$calls" ]]
[[ "$(cat "$temporary_home/Applications/Aquilum.app/Keep.txt")" == 'installed app remains' ]]

# TEST-ONLY COPY: replace precisely TWO source literals in an isolated file.
# No production environment override or release-supplied signing root exists.
# This runs the same Bash installer, with synthetic pinned identity for the
# positive offline fixture, against mocks (not a genuinely signed DMG).
grep -Fxq 'AKULA_DEVELOPER_TEAM_ID=""' "$script_dir/install.sh"
grep -Fxq 'AKULA_BUNDLE_ID=""' "$script_dir/install.sh"
test_installer="$temporary_home/test-only-signed-akula-installer.sh"
sed -e 's/^AKULA_DEVELOPER_TEAM_ID=""$/AKULA_DEVELOPER_TEAM_ID="AB12CD34EF"/' \
    -e 's/^AKULA_BUNDLE_ID=""$/AKULA_BUNDLE_ID="com.lalalalendia.akula"/' \
    "$script_dir/install.sh" > "$test_installer"
grep -Fxq 'AKULA_DEVELOPER_TEAM_ID="AB12CD34EF"' "$test_installer"
grep -Fxq 'AKULA_BUNDLE_ID="com.lalalalendia.akula"' "$test_installer"

if HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" AKULA_INSTALL_CURL_CALLS="$calls" \
    bash "$test_installer" > /dev/null 2> "$error_log"; then
  echo 'Test fixture unexpectedly succeeded without a fork release' >&2
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


# Offline macOS installer acceptance: execute the test-only copy of the real
# script (with two synthetic hard-coded pin values), while mocking external OS
# utilities. The production script is separately proved fail-closed above.
cat > "$temporary_home/tools/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$AKULA_INSTALL_CURL_CALLS"
if [[ "${AKULA_TEST_CURL_PAUSE:-0}" == 1 && "$*" == *"/releases/latest"* ]]; then
  touch "$AKULA_TEST_CURL_READY"
  until [[ -e "$AKULA_TEST_CURL_CONTINUE" ]]; do sleep 0.05; done
fi
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
[[ "$#" -eq 6 && "$1" == --verify && "$2" == --deep &&
   "$3" == --strict && "$4" == -R ]]
# The code under test must require Apple chain, Developer ID APPLICATION OID,
# issuer extension, pinned Team ID and bundle ID (not just codesign integrity).
[[ "$5" == '=anchor apple generic '* ]]
[[ "$5" == *'identifier "com.lalalalendia.akula"'* ]]
[[ "$5" == *'certificate 1[field.1.2.840.113635.100.6.2.6] exists'* ]]
[[ "$5" == *'certificate leaf[field.1.2.840.113635.100.6.1.13] exists'* ]]
[[ "$5" == *'certificate leaf[subject.OU] = "AB12CD34EF"'* ]]
[[ -f "$6/Contents/MacOS/aquilum-app" ]]
EOF
cat > "$temporary_home/tools/spctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -eq 4 && "$1" == --assess && "$2" == --type && "$3" == execute ]]
if [[ "${AKULA_TEST_NOTARIZATION_FAIL:-0}" == 1 ]]; then exit 39; fi
[[ -f "$4/Contents/MacOS/aquilum-app" ]]
EOF
cat > "$temporary_home/tools/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${AKULA_TEST_KILL_AFTER_BACKUP:-0}" == 1
   && "$1" == "$HOME/Applications/Aquilum.app"
   && "$2" == "$HOME/Applications"/.akula-install.*/Aquilum.previous.app ]]; then
  /bin/mv "$@"
  kill -KILL "$PPID"
  exit 137
fi
if [[ "${AKULA_TEST_KILL_AFTER_PROMOTION:-0}" == 1
   && "$1" == "$HOME/Applications"/.akula-install.*/Aquilum.app
   && "$2" == "$HOME/Applications/Aquilum.app" ]]; then
  /bin/mv "$@"
  kill -KILL "$PPID"
  exit 137
fi
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
    bash "$test_installer" > "$temporary_home/install-stdout.log" 2> "$error_log"
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
grep -Fq 'failed pinned Akula Developer ID signature checks' "$error_log"
unset AKULA_TEST_CODESIGN_FAIL

export AKULA_TEST_NOTARIZATION_FAIL=1
must_fail_without_changing_old
grep -Fq 'failed macOS security assessment' "$error_log"
unset AKULA_TEST_NOTARIZATION_FAIL

export AKULA_TEST_PROMOTION_FAIL=1
must_fail_without_changing_old
grep -Fq 'restoring previous installation' "$error_log"
unset AKULA_TEST_PROMOTION_FAIL


# Both installer and uninstaller use the same kernel lock. A held descriptor
# must cause immediate refusal without touching the active application.
exec 8>>"$temporary_home/Applications/.akula-install.lock"
PATH="$temporary_home/tools:$PATH" lockf -s -t 0 8
if run_installer; then
  echo 'Installer bypassed another process holding the lock' >&2
  exit 1
fi
grep -Fq 'holds the Applications lock' "$error_log"
if HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" \
    bash "$script_dir/uninstall.sh" 2> "$error_log"; then
  echo 'Uninstaller bypassed the held shared lock' >&2
  exit 1
fi
grep -Fq 'holds the Applications lock' "$error_log"
old_intact
exec 8>&-

# Test genuine parallel installers: first one is paused inside its API fetch,
# second one must refuse before even fetching a release.
export AKULA_TEST_CURL_PAUSE=1
export AKULA_TEST_CURL_READY="$temporary_home/first-paused"
export AKULA_TEST_CURL_CONTINUE="$temporary_home/first-continue"
run_installer &
first_pid=$!
for attempt in {1..100}; do
  [[ -e "$AKULA_TEST_CURL_READY" ]] && break
  sleep 0.05
done
if [[ ! -e "$AKULA_TEST_CURL_READY" ]]; then
  echo 'First installer failed to reach paused download state' >&2
  touch "$AKULA_TEST_CURL_CONTINUE"
  wait "$first_pid" || true
  exit 1
fi
if run_installer; then
  echo 'Second independent installer bypassed the first' >&2
  touch "$AKULA_TEST_CURL_CONTINUE"
  wait "$first_pid" || true
  exit 1
fi
grep -Fq 'holds the Applications lock' "$error_log"
touch "$AKULA_TEST_CURL_CONTINUE"
wait "$first_pid"
unset AKULA_TEST_CURL_PAUSE AKULA_TEST_CURL_READY AKULA_TEST_CURL_CONTINUE
no_staging_left
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'new installed version' ]]

# Kill installer AFTER moving old.app to backup but BEFORE promoting staged
# app. Kernel lock is automatically released; next launch restores backup
# before any network request, then stops rather than blindly retrying.
printf 'old installed version\n' > "$existing/Contents/MacOS/aquilum-app"
printf 'installed app remains\n' > "$existing/Keep.txt"
export AKULA_TEST_KILL_AFTER_BACKUP=1
if run_installer; then
  echo 'Fatal interruption after backup unexpectedly succeeded' >&2
  exit 1
fi
unset AKULA_TEST_KILL_AFTER_BACKUP
[[ ! -e "$existing" ]]
backup="$(find "$temporary_home/Applications" -maxdepth 2 -type d -name 'Aquilum.previous.app' -print -quit)"
[[ -n "$backup" && -f "$backup/Contents/MacOS/aquilum-app" ]]
[[ "$(cat "$backup/Contents/MacOS/aquilum-app")" == 'old installed version' ]]
if HOME="$temporary_home" PATH="$temporary_home/tools:$PATH" \
    bash "$script_dir/uninstall.sh" 2> "$error_log"; then
  echo 'Uninstall ignored an interrupted backup' >&2
  exit 1
fi
grep -Fq 'needs review before uninstall' "$error_log"
if run_installer; then
  echo 'Recovery must not automatically start a new installation' >&2
  exit 1
fi
grep -Fq 'Recovered interrupted install' "$error_log"
old_intact
no_staging_left

# Kill AFTER new.app was promoted but BEFORE deleting prior app. Both
# generations exist. It is unsafe to auto-delete the backup; fail closed.
export AKULA_TEST_KILL_AFTER_PROMOTION=1
if run_installer; then
  echo 'Fatal interruption after promotion unexpectedly succeeded' >&2
  exit 1
fi
unset AKULA_TEST_KILL_AFTER_PROMOTION
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'new installed version' ]]
old_backup="$(find "$temporary_home/Applications" -maxdepth 2 -type d -name 'Aquilum.previous.app' -print -quit)"
[[ -n "$old_backup" ]]
[[ "$(cat "$old_backup/Contents/MacOS/aquilum-app")" == 'old installed version' ]]
if run_installer; then
  echo 'Ambiguous generation was silently accepted' >&2
  exit 1
fi
grep -Fq 'needs review' "$error_log"
[[ "$(cat "$old_backup/Contents/MacOS/aquilum-app")" == 'old installed version' ]]
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'new installed version' ]]
[[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]
# Test-only cleanup: production requires a human to resolve both generations.
rm -rf "$(dirname "$old_backup")"
printf 'old installed version\n' > "$existing/Contents/MacOS/aquilum-app"
printf 'installed app remains\n' > "$existing/Keep.txt"

# Two different abandoned stages with a missing destination are not a proof
# of which old app should be restored. Both remain untouched on HOLD.
stage1="$temporary_home/Applications/.akula-install.11111111"
stage2="$temporary_home/Applications/.akula-install.22222222"
mkdir -p "$stage1" "$stage2"
/bin/mv "$existing" "$stage1/Aquilum.previous.app"
cp -R "$stage1/Aquilum.previous.app" "$stage2/Aquilum.previous.app"
if run_installer; then
  echo 'Ambiguous concurrent interrupted generations were accepted' >&2
  exit 1
fi
grep -Fq 'Multiple interrupted install generations' "$error_log"
[[ ! -e "$existing" ]]
[[ -f "$stage1/Aquilum.previous.app/Contents/MacOS/aquilum-app" ]]
[[ -f "$stage2/Aquilum.previous.app/Contents/MacOS/aquilum-app" ]]
# Test fixture only: select first backup manually, then remove staging.
/bin/mv "$stage1/Aquilum.previous.app" "$existing"
rm -rf "$stage1" "$stage2"
old_intact
no_staging_left

# Recovery may restore the old app, but must not recursively discard an
# unexpected file placed in its abandoned stage.
stage3="$temporary_home/Applications/.akula-install.33333333"
mkdir -p "$stage3"
/bin/mv "$existing" "$stage3/Aquilum.previous.app"
printf 'do not delete\n' > "$stage3/unexpected-user.txt"
if run_installer; then
  echo 'Recovery deleted unknown contents of abandoned stage' >&2
  exit 1
fi
grep -Fq 'unverified entries' "$error_log"
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'old installed version' ]]
[[ -f "$stage3/unexpected-user.txt" ]]
[[ "$(cat "$stage3/unexpected-user.txt")" == 'do not delete' ]]
# Test fixture only: manual inspection is complete.
rm -rf "$stage3"
old_intact
no_staging_left

run_installer
[[ "$(cat "$existing/Contents/MacOS/aquilum-app")" == 'new installed version' ]]
[[ ! -e "$existing/Keep.txt" ]]
[[ "$(cat "$temporary_home/Notes/Keep.md")" == keep ]]
no_staging_left
