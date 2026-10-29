#!/usr/bin/env bash
# B1c1 Ubuntu guest only: transient DynamicUser runs ACTUAL B1a signed,
# owner-CAS committed fixed-path ELF. This is not a production unit.
set -euo pipefail
umask 077
test "$(id -u)" -eq 0
test -f /etc/akula-b1b2b-lab.marker
test "$#" -eq 1
root=/run/akula-b1c0-one-shot-ci
file="$root/target/node"
# Test-only, root-controlled executable location. /run is runtime state and
# may be mounted noexec. Never relax that mount or systemd sandbox.
exec_root=/opt/akula-b1c1-committed-lab
exec_file="$exec_root/node"
exec_marker="$exec_root/enrollment"
node_source=/home/akulavm/b1b2b/v1.node
wrapper=/opt/akula-b1b2b-lab/wrapper
vault=/srv/akula-b1b2b-test-vault
credential=/etc/akula-b1b2b-lab-token
unit=akula-b1c1-cas-committed-node
port=23459
url="http://127.0.0.1:$port"
refuse(){ echo "B1C1_VM_REFUSED: $*" >&2; exit 1; }
enrolled(){
  test ! -L "$root" && test -d "$root" || refuse "no fixed root"
  test "$(stat -c '%u:%a' "$root")" = "0:711" || refuse "bad root"
  test "$(stat -c '%u:%a' "$root/target")" = "0:755" || refuse "bad target"
  test "$(stat -c '%u:%a:%h' "$root/enrollment")" = "0:600:1" || refuse "bad marker"
  cmp -s "$root/enrollment" <(printf 'AKULA-B1C0-ENROLLED-CI-V1\n') ||
    refuse "untrusted enrollment"
}
case "$1" in
  activate)
    enrolled
    test -f "$file" && test ! -L "$file" || refuse "no pinned ELF"
    test "$(stat -c '%u:%a:%h' "$file")" = "0:555:1" ||
      refuse "bad signed executable mode"
    test ! -e "$root/target/node.pending" || refuse "pending transaction"
    cmp -s "$node_source" "$file" || refuse "different ELF than signed VM release"
    test -x "$wrapper" && test -f "$credential" || refuse "missing fixed lab runtime"
    echo "B1C1_CAS_FILE_MOUNT_OPTIONS=$(findmnt -no OPTIONS -T "$file")"
    # The B1c0 transaction is fixed in /run. Its inode and CAS commit have
    # already been rechecked. Root explicitly stages the SAME verified bytes
    # to a separate, fixed, executable-path laboratory under /opt. This copy
    # is NOT an atomic/durable production install transaction.
    test ! -e "$exec_root" && test ! -L "$exec_root" ||
      refuse "preexisting executable installation directory"
    install -d -o root -g root -m 0755 "$exec_root"
    printf 'AKULA-B1C1-CI-EXEC-COPY-V1\n' > "$exec_marker"
    chmod 0600 "$exec_marker"
    test "$(stat -c '%u:%a:%h' "$exec_marker")" = "0:600:1" ||
      refuse "untrusted executable enrollment"
    install -o root -g root -m 0555 "$file" "$exec_file"
    test ! -L "$exec_file" &&
      test "$(stat -c '%u:%a:%h' "$exec_file")" = "0:555:1" ||
      refuse "executable copy not immutable and root-owned"
    cmp -s "$node_source" "$exec_file" &&
      cmp -s "$file" "$exec_file" || refuse "exec copy differs from CAS ELF"
    cas_sha="$(sha256sum "$file" | cut -d ' ' -f1)"
    exec_sha="$(sha256sum "$exec_file" | cut -d ' ' -f1)"
    [[ "$cas_sha" =~ ^[0-9a-f]{64}$ && "$exec_sha" == "$cas_sha" ]] ||
      refuse "exec copy differs from pinned CAS sha256"
    sync "$exec_file" "$exec_root"
    exec_opts="$(findmnt -no OPTIONS -T "$exec_file")"
    case ",$exec_opts," in
      *,noexec,*) refuse "fixed executable mount is noexec" ;;
    esac
    echo "B1C1_EXECUTABLE_HANDOFF_SHA256_PASS"
    if systemctl is-active --quiet "$unit"; then
      refuse "pre-existing active unit"
    fi
    # Temporary unit, NO enable, no autostart, no install/write IPC.
    systemd-run --unit="$unit" --service-type=simple \
      --property=DynamicUser=yes \
      --property=StateDirectory=akula-b1c1-cas-node-state \
      --property=StateDirectoryMode=0700 \
      --property=NoNewPrivileges=yes \
      --property=ProtectSystem=strict \
      --property=ProtectHome=yes \
      --property=PrivateTmp=yes \
      --property=PrivateDevices=yes \
      --property=ProtectKernelTunables=yes \
      --property=ProtectKernelModules=yes \
      --property=ProtectControlGroups=yes \
      --property=RestrictAddressFamilies='AF_INET AF_UNIX' \
      --property=IPAddressDeny=any \
      --property=IPAddressAllow=localhost \
      --property=ReadOnlyPaths="$vault" \
      --property=LoadCredential="node-token:$credential" \
      "$wrapper" "$exec_file" "$vault" "$port"
    trap 'systemctl stop "$unit" >/dev/null 2>&1 || true' EXIT
    good=false
    for _ in $(seq 1 60); do
      status="$(curl --noproxy '*' -s --max-time 2 --connect-timeout 1 \
        -H "Authorization: Bearer $(cat "$credential")" \
        -o /tmp/akula-b1c1-health -w '%{http_code}' "$url/health" || true)"
      if [[ "$status" == 200 ]] &&
          grep -Fq '"status":"ok"' /tmp/akula-b1c1-health; then
        good=true
        break
      fi
      if ! systemctl is-active --quiet "$unit"; then
        journalctl -u "$unit" -o cat -n 80 --no-pager >&2 || true
        refuse "committed Node service crashed"
      fi
      sleep 0.2
    done
    [[ "$good" == true ]] || refuse "signed committed Node not healthy"
    test "$(systemctl show "$unit" -p DynamicUser --value)" = yes
    test "$(systemctl show "$unit" -p ProtectSystem --value)" = strict
    unauthorized="$(curl --noproxy '*' -s --max-time 3 \
      -o /tmp/akula-b1c1-anon -w '%{http_code}' "$url/v1/note" || true)"
    [[ "$unauthorized" == 401 ]] || refuse "anonymous read not forbidden"
    authorized="$(curl --noproxy '*' -s --max-time 3 \
      -H "Authorization: Bearer $(cat "$credential")" \
      -o /tmp/akula-b1c1-read -w '%{http_code}' "$url/v1/note" || true)"
    [[ "$authorized" == 200 ]] &&
      grep -Fq 'b1b2b signed-vm test note' /tmp/akula-b1c1-read ||
      refuse "signed committed ELF did not read test note"
    echo B1C1_QEMU_ROOT_COMMITTED_DYNAMICUSER_NODE_PASS
    ;;
  cleanup)
    enrolled
    systemctl stop "$unit" >/dev/null 2>&1 || true
    systemctl reset-failed "$unit" >/dev/null 2>&1 || true
    # Never recursively delete a guessed /opt path. Verify root-owned
    # enrollment and only remove exact expected names in disposable QEMU.
    test ! -L "$exec_root" &&
      test "$(stat -c '%u:%a' "$exec_root")" = "0:755" ||
      refuse "untrusted executable installation root"
    test "$(stat -c '%u:%a:%h' "$exec_marker")" = "0:600:1" &&
      cmp -s "$exec_marker" <(printf 'AKULA-B1C1-CI-EXEC-COPY-V1\n') ||
      refuse "untrusted executable enrollment"
    test ! -L "$exec_file" &&
      test "$(stat -c '%u:%a:%h' "$exec_file")" = "0:555:1" ||
      refuse "untrusted executable file"
    cmp -s "$file" "$exec_file" || refuse "executable changed before cleanup"
    rm -- "$exec_file" "$exec_marker"
    rmdir -- "$exec_root"
    test ! -e "$root/target/node.pending" || refuse "unexpected pending state"
    rm -- "$root/target/node" "$root/enrollment"
    rmdir -- "$root/target" "$root"
    test ! -e "$root" && test ! -e "$exec_root" ||
      refuse "disposable namespace cleanup incomplete"
    echo B1C1_QEMU_CI_NAMESPACE_CLEANUP_PASS
    ;;
  *) refuse "unsupported action" ;;
esac
