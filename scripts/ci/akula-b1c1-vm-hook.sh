#!/usr/bin/env bash
# Sourced only inside existing disposable B1b2b QEMU host lab.
# Uses the existing pinned SSH host key and isolated guest.
set -euo pipefail
[[ "$AKULA_B1C1_VM_BRIDGE" == "SIGNED_ELF_TO_ROOT_CAS" ]]
[[ "$GITHUB_ACTIONS" == "true" && "$RUNNER_OS" == "Linux" ]]
test -f "$lab/v1.node" && test -f "$lab/v1.manifest" && test -f "$lab/v1.sig"
test -x .artifacts/cargo-native/debug/examples/akula_b1b2b_release_fixture
signed_binary_count=0
for testbin in .artifacts/cargo-native/debug/deps/server_bootstrap_b1c0-*; do
  if [[ -f "$testbin" && -x "$testbin" && "$testbin" != *.d ]]; then
    cp -- "$testbin" "$lab/b1c1-commit-test"
    strip --strip-debug "$lab/b1c1-commit-test"
    chmod 0555 "$lab/b1c1-commit-test"
    signed_binary_count=$((signed_binary_count + 1))
  fi
done
test "$signed_binary_count" -eq 1
# Verify pre-boot trust key and EXACT signed B1a release first.
signed_pub_sha="$(sha256sum "$lab/v1.pub" | cut -d ' ' -f1)"
ssh_lab "test \"\$(sudo sha256sum /etc/akula-b1b2b-lab-public.ed25519 | cut -d ' ' -f1)\" = '$signed_pub_sha'"
ssh_lab "sudo /home/akulavm/b1b2b/b1a-fixture verify \
  /home/akulavm/b1b2b/v1.node /home/akulavm/b1b2b/v1.manifest \
  /home/akulavm/b1b2b/v1.sig /etc/akula-b1b2b-lab-public.ed25519"
scp -q -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o "UserKnownHostsFile=$lab/known_hosts" -o GlobalKnownHostsFile=/dev/null \
  -o HostKeyAlias=akula-b1b2b -o HostKeyAlgorithms=ssh-ed25519 \
  -o PasswordAuthentication=no -o KbdInteractiveAuthentication=no \
  -o IdentityAgent=none -o IdentitiesOnly=yes -P "$ssh_port" -i "$lab/client" \
  "$lab/b1c1-commit-test" scripts/ci/akula-b1c1-guest-lab.sh \
  akulavm@127.0.0.1:/home/akulavm/b1b2b/
bridge_output="$(ssh_lab "cd /home/akulavm/b1b2b &&
  chmod 0555 b1c1-commit-test &&
  sudo env GITHUB_ACTIONS=true RUNNER_OS=Linux \
    AKULA_B1C1_VM_PROOF=SIGNED_ELF_TO_FIXED_NODE_CAS_ONLY \
    ./b1c1-commit-test ci_real_preprovisioned_signed_elf_b1c1 --exact --nocapture")"
printf '%s\n' "$bridge_output"
grep -Fq 'B1C1_GUEST_ROOT_PINNED_ACTUAL_SIGNED_ELF_CAS_COMMIT_PASS' <<< "$bridge_output"
expected_sha="$(sha256sum "$lab/v1.node" | cut -d ' ' -f1)"
committed_sha="$(ssh_lab 'sudo sha256sum /run/akula-b1c0-one-shot-ci/target/node' | cut -d ' ' -f1)"
[[ "$committed_sha" == "$expected_sha" ]]
echo "B1C1_QEMU_SAME_ORIGINAL_SIGNED_ELF_CAS_HASH_PASS"
guest_result="$(ssh_lab 'sudo bash /home/akulavm/b1b2b/akula-b1c1-guest-lab.sh activate')"
printf '%s\n' "$guest_result"
grep -Fq 'B1C1_EXECUTABLE_HANDOFF_SHA256_PASS' <<< "$guest_result"
grep -Fq 'B1C1_QEMU_ROOT_COMMITTED_DYNAMICUSER_NODE_PASS' <<< "$guest_result"
ssh_lab 'sudo bash /home/akulavm/b1b2b/akula-b1c1-guest-lab.sh cleanup'
echo "B1C1_QEMU_CAS_PINNED_SIGNED_REAL_NODE_END_TO_END_PASS"
