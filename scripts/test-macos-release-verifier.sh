#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
verifier="$repo_root/scripts/verify-macos-release.sh"
production="$repo_root/scripts/install.sh"
test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT
mkdir -p "$test_root/tools"
dmg="$test_root/Aquilum_release_aarch64.dmg"
printf 'synthetic, NOT a signed DMG\n' > "$dmg"
# Synthetic archive must match the fake mounted DMG exactly, including modes.
fixture="$test_root/updater-fixture/Aquilum.app"
archive="$test_root/Aquilum_aarch64.app.tar.gz"
mkdir -p "$fixture/Contents/MacOS"
printf 'simulated executable\n' > "$fixture/Contents/MacOS/aquilum-app"
printf 'plist\n' > "$fixture/Contents/Info.plist"
chmod 755 "$fixture/Contents/MacOS/aquilum-app"
python3 - "$fixture" "$archive" <<'PY'
import sys
import tarfile
with tarfile.open(sys.argv[2], 'w:gz') as output:
    output.add(sys.argv[1], arcname='Aquilum.app')
PY
calls="$test_root/tools.log"
errors="$test_root/errors.log"
export AKULA_RELEASE_TEST_CALLS="$calls"

cat > "$test_root/tools/uname" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == -s ]] || exit 91
printf 'Darwin\n'
EOF
cat > "$test_root/tools/xcrun" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'stapler %s\n' "$*" >> "$AKULA_RELEASE_TEST_CALLS"
[[ "$#" -eq 3 && "$1" == stapler && "$2" == validate ]] || exit 90
[[ "${AKULA_TEST_BAD_STAPLE:-0}" != 1 ]] || exit 33
EOF
cat > "$test_root/tools/hdiutil" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'hdiutil %s\n' "$*" >> "$AKULA_RELEASE_TEST_CALLS"
if [[ "$1" == attach ]]; then
  last=''
  mount=''
  for token in "$@"; do
    if [[ "$last" == -mountpoint ]]; then mount="$token"; fi
    last="$token"
  done
  [[ -n "$mount" ]] || exit 91
  mkdir -p "$mount/Aquilum.app/Contents/MacOS"
  printf 'simulated executable\n' > "$mount/Aquilum.app/Contents/MacOS/aquilum-app"
  printf 'plist\n' > "$mount/Aquilum.app/Contents/Info.plist"
  chmod 755 "$mount/Aquilum.app/Contents/MacOS/aquilum-app"
elif [[ "$1" == detach ]]; then
  [[ -d "$2" ]] || exit 92
  rm -rf "$2/Aquilum.app"
else
  exit 93
fi
EOF
cat > "$test_root/tools/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'codesign %s\n' "$*" >> "$AKULA_RELEASE_TEST_CALLS"
[[ "${AKULA_TEST_BAD_CODESIGN:-0}" != 1 ]] || exit 32
[[ "$#" -eq 6 && "$1" == --verify && "$2" == --deep &&
   "$3" == --strict && "$4" == -R ]] || exit 91
[[ "$5" == '=anchor apple generic '* ]] || exit 92
[[ "$5" == *'identifier "com.lalalalendia.akula"'* ]] || exit 93
[[ "$5" == *'certificate 1[field.1.2.840.113635.100.6.2.6] exists'* ]] || exit 94
[[ "$5" == *'certificate leaf[field.1.2.840.113635.100.6.1.13] exists'* ]] || exit 95
[[ "$5" == *'certificate leaf[subject.OU] = "AB12CD34EF"'* ]] || exit 96
[[ -f "$6/Contents/MacOS/aquilum-app" ]] || exit 97
EOF
cat > "$test_root/tools/spctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'spctl %s\n' "$*" >> "$AKULA_RELEASE_TEST_CALLS"
[[ "${AKULA_TEST_BAD_SPCTL:-0}" != 1 ]] || exit 44
[[ "$#" -eq 4 && "$1" == --assess &&
   "$2" == --type && "$3" == execute ]] || exit 91
[[ -f "$4/Contents/MacOS/aquilum-app" ]] || exit 92
EOF
chmod +x "$test_root/tools/"*

verify() { PATH="$test_root/tools:$PATH" bash "$verifier" "$1" "$2" "$archive" > "$test_root/stdout.log" 2> "$errors"; }
expect_hold() {
  if verify "$1" "$2"; then
    printf 'Untrusted release unexpectedly passed: %s\n' "$3" >&2
    exit 1
  fi
  grep -Fq "$3" "$errors"
}

# Real installer source has deliberately NO provisioned signing identity.
grep -Fxq 'AKULA_DEVELOPER_TEAM_ID=""' "$production"
grep -Fxq 'AKULA_BUNDLE_ID=""' "$production"
expect_hold "$production" "$dmg" 'Developer ID Team is not provisioned'
[[ ! -e "$calls" ]] # Must reject *before* stapler, mount, codesign, spctl.

# Only a disposable test copy receives synthetic static pins. Never modify
# the release script itself or inject publisher credentials through the env.
mock_installer="$test_root/mock-pinned-source.sh"
sed -e 's/^AKULA_DEVELOPER_TEAM_ID=""$/AKULA_DEVELOPER_TEAM_ID="AB12CD34EF"/' \
    -e 's/^AKULA_BUNDLE_ID=""$/AKULA_BUNDLE_ID="com.lalalalendia.akula"/' \
    "$production" > "$mock_installer"
verify "$mock_installer" "$dmg"
grep -Fq 'pinned publisher, notarized DMG and updater TAR parity verified' "$test_root/stdout.log"
grep -Fq 'stapler validate' "$calls"
grep -Fq 'hdiutil attach' "$calls"
grep -Fq 'codesign --verify --deep --strict -R' "$calls"
grep -Fq 'spctl --assess --type execute' "$calls"
grep -Fq 'hdiutil detach' "$calls"
: > "$calls"

# Valid notarization and publisher mocks must not admit a tampered updater
# TAR even though the DMG itself remains identical and code-signed.
printf 'not a gzip stream\n' > "$test_root/tampered.app.tar.gz"
if PATH="$test_root/tools:$PATH" bash "$verifier" "$mock_installer" "$dmg" "$test_root/tampered.app.tar.gz" > /dev/null 2> "$errors"; then
  echo 'Tampered updater archive passed after notarized DMG verification' >&2
  exit 1
fi
grep -Fq 'Updater TAR and notarized DMG bundles differ' "$errors"
grep -Fq 'hdiutil detach' "$calls"
: > "$calls"

export AKULA_TEST_BAD_STAPLE=1
expect_hold "$mock_installer" "$dmg" 'no valid stapled Apple notarization ticket'
unset AKULA_TEST_BAD_STAPLE
if grep -Fq 'hdiutil attach' "$calls"; then
  echo 'Unsigned DMG was mounted before notarization acceptance' >&2
  exit 1
fi
: > "$calls"

export AKULA_TEST_BAD_CODESIGN=1
expect_hold "$mock_installer" "$dmg" 'not signed by the pinned Akula'
unset AKULA_TEST_BAD_CODESIGN
grep -Fq 'hdiutil detach' "$calls"
: > "$calls"

export AKULA_TEST_BAD_SPCTL=1
expect_hold "$mock_installer" "$dmg" 'did not pass macOS Gatekeeper'
unset AKULA_TEST_BAD_SPCTL
grep -Fq 'hdiutil detach' "$calls"
: > "$calls"

bad_team="$test_root/bad-team.sh"
sed 's/^AKULA_DEVELOPER_TEAM_ID="AB12CD34EF"$/AKULA_DEVELOPER_TEAM_ID=""/' \
  "$mock_installer" > "$bad_team"
expect_hold "$bad_team" "$dmg" 'Developer ID Team is not provisioned'
[[ ! -s "$calls" ]]

# Fail on multiple contradictory source pins rather than picking one.
ambiguous="$test_root/ambiguous.sh"
cp "$mock_installer" "$ambiguous"
printf 'AKULA_DEVELOPER_TEAM_ID="ZX12CD34EF"\n' >> "$ambiguous"
expect_hold "$ambiguous" "$dmg" 'missing or ambiguous pinned'
[[ ! -s "$calls" ]]

ln -s "$dmg" "$test_root/alias.dmg"
expect_hold "$mock_installer" "$test_root/alias.dmg" 'DMG absent or a symlink'
[[ ! -s "$calls" ]]

printf 'Verified macOS release fail-closed and mock positive/negative matrix\n'
