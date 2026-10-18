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
