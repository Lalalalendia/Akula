#!/usr/bin/env bash
set -euo pipefail

# Android-emulator-runner executes its "script" through /bin/sh, often splitting
# YAML lines. Keep discovery, transfer and assertions in this single Bash file.
adb wait-for-device

repo_root="$(git rev-parse --show-toplevel)"
tests_dir="$repo_root/staya/roha-transport/target/x86_64-linux-android/debug/deps"
test_binary="$(find "$tests_dir" -maxdepth 1 -type f -name 'peer_transfer-*' -perm /111 -print -quit)"
if [[ -z "$test_binary" ]]; then
  echo "No Android peer_transfer test executable found in $tests_dir" >&2
  exit 1
fi

adb push "$test_binary" /data/local/tmp/akula_staya_peer_test
adb shell chmod 755 /data/local/tmp/akula_staya_peer_test

echo "Android API35: test permitted QUIC bytes, unknown outbound identity and unknown inbound identity"
adb shell "/data/local/tmp/akula_staya_peer_test --nocapture --test-threads=1" \
  | tee /tmp/akula-staya-emulator.log
grep -F -q "test result: ok. 3 passed; 0 failed" /tmp/akula-staya-emulator.log
for case in \
  permitted_peers_exchange_verified_bytes \
  outbound_connection_rejects_unknown_peer_before_network_io \
  inbound_unknown_peer_is_closed_without_blocking_admitted_peer; do
  grep -F -q "test $case ... ok" /tmp/akula-staya-emulator.log
done

adb shell rm -f /data/local/tmp/akula_staya_peer_test
echo "PASS: 3 Android emulator native Staya adapter tests with exact pinned Roha/noq"
