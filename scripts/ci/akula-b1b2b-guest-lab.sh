#!/usr/bin/env bash
# Disposable Ubuntu CI VM only. No production installer or owner grant.
set -euo pipefail
umask 077
test "$(id -u)" -eq 0
test -f /etc/akula-b1b2b-lab.marker
test "$#" -eq 1
PREFIX=/opt/akula-b1b2b-lab
TRUST=/etc/akula-b1b2b-lab-public.ed25519
UPLOAD=/home/akulavm/b1b2b
VERIFY="$UPLOAD/b1a-fixture"
UNIT=/etc/systemd/system/akula-b1b2b-lab.service
SERVICE=akula-b1b2b-lab.service
FIXTURE=/srv/akula-b1b2b-test-vault
CREDENTIAL=/run/akula-b1b2b-lab-token
URL=http://127.0.0.1:23457
test -s "$TRUST" && test -x "$VERIFY" && test -d "$UPLOAD"
fail(){ echo "B1B2B_VM_REFUSED: $*" >&2; exit 1; }
absent(){ test ! -e "$1" && test ! -L "$1" || fail "pre-existing path $1"; }
health(){
  local status
  for i in $(seq 1 30); do
    status=$(curl --noproxy '*' -s -m 2 --connect-timeout 1 \
      -H "Authorization: Bearer $(cat "$CREDENTIAL")" \
      -o /tmp/akula-b1b2b-health -w '%{http_code}' "$URL/health" || true)
    if [[ "$status" = 200 ]] && grep -Fq '"status":"ok"' /tmp/akula-b1b2b-health; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}
read_fixture(){
  local unauth auth
  unauth=$(curl --noproxy '*' -s -m 3 -o /tmp/akula-b1b2b-anon \
    -w '%{http_code}' "$URL/v1/note" || true)
  test "$unauth" = 401 || fail "unauthenticated access allowed"
  auth=$(curl --noproxy '*' -s -m 3 \
    -H "Authorization: Bearer $(cat "$CREDENTIAL")" \
    -o /tmp/akula-b1b2b-read -w '%{http_code}' "$URL/v1/note" || true)
  test "$auth" = 200 && grep -Fq "b1b2b signed-vm test note" /tmp/akula-b1b2b-read \
    || fail "authorized read refused"
}
verify_pkg(){
  "$VERIFY" verify "$UPLOAD/$1.node" "$UPLOAD/$1.manifest" "$UPLOAD/$1.sig" "$TRUST"
}
atomic_link(){
  local candidate="$1" tmp="$PREFIX/active.next.$$"
  absent "$tmp"
  ln -s "releases/$candidate" "$tmp"
  mv -T -- "$tmp" "$PREFIX/active"
  sync "$PREFIX"
}
rollback(){
  atomic_link 0001
  systemctl restart "$SERVICE"
  health || fail "old signed release cannot restart"
  read_fixture
  rm -f -- "$PREFIX/in-progress"
  sync "$PREFIX"
}
case "$1" in
  bootstrap)
    for p in "$PREFIX" "$FIXTURE" "$UNIT" "$CREDENTIAL"; do absent "$p"; done
    verify_pkg v1
    install -d -m 0755 "$PREFIX" # root-owned, traversable by DynamicUser; not writable
    install -d -m 0755 "$PREFIX/releases" "$PREFIX/releases/0001"
    install -m 0555 "$UPLOAD/v1.node" "$PREFIX/releases/0001/node"
    install -m 0555 "$UPLOAD/wrapper" "$PREFIX/wrapper"
    "$VERIFY" verify "$PREFIX/releases/0001/node" "$UPLOAD/v1.manifest" \
      "$UPLOAD/v1.sig" "$TRUST"
    chmod 0555 "$PREFIX/releases/0001"
    install -d -m 0755 "$FIXTURE"
    # Unpredictable O_EXCL temp name avoids collisions and symlink pre-creation.
    fixture_temp="$(mktemp /tmp/akula-b1b2b-note.XXXXXXXX)"
    printf 'b1b2b signed-vm test note\n' > "$fixture_temp"
    install -m 0444 "$fixture_temp" "$FIXTURE/demo.md"
    chmod 0555 "$FIXTURE"
    rm -f -- "$fixture_temp"
    install -m 0600 "$UPLOAD/node-token" "$CREDENTIAL"
    ln -s releases/0001 "$PREFIX/active"
    cat > "$UNIT" <<'UNIT_FILE'
[Unit]
Description=Akula B1b2b disposable signed package proof NOT PRODUCTION
After=network.target
[Service]
Type=simple
DynamicUser=yes
StateDirectory=akula-b1b2b-lab-state
StateDirectoryMode=0700
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_INET AF_UNIX
IPAddressDeny=any
IPAddressAllow=localhost
ReadOnlyPaths=/srv/akula-b1b2b-test-vault
UMask=0077
LoadCredential=node-token:/run/akula-b1b2b-lab-token
ExecStart=/opt/akula-b1b2b-lab/wrapper /opt/akula-b1b2b-lab/active/node /srv/akula-b1b2b-test-vault 23457
Restart=no
[Install]
WantedBy=multi-user.target
UNIT_FILE
    chmod 0644 "$UNIT"
    sync "$PREFIX" "$UNIT"
    systemctl daemon-reload
    systemctl start "$SERVICE"
    health || fail "signed v1 Node not healthy"
    read_fixture
    test "$(systemctl show "$SERVICE" -p DynamicUser --value)" = yes
    test "$(systemctl show "$SERVICE" -p ProtectSystem --value)" = strict
    test "$(readlink "$PREFIX/active")" = releases/0001
    echo "B1B2B_VM_SIGNED_V1_SYSTEMD_PASS"
    ;;
  negative)
    verify_pkg v2
    cp "$UPLOAD/v2.node" "$UPLOAD/tampered.node"
    printf x >> "$UPLOAD/tampered.node"
    if "$VERIFY" verify "$UPLOAD/tampered.node" "$UPLOAD/v2.manifest" \
        "$UPLOAD/v2.sig" "$TRUST" >/dev/null 2>&1; then
      fail "tampered signed package accepted"
    fi
    install -d -m 0755 "$PREFIX/releases/0002"
    install -m 0555 "$UPLOAD/v2.node" "$PREFIX/releases/0002/node"
    "$VERIFY" verify "$PREFIX/releases/0002/node" "$UPLOAD/v2.manifest" \
      "$UPLOAD/v2.sig" "$TRUST"
    chmod 0555 "$PREFIX/releases/0002"
    test ! -e "$PREFIX/in-progress" || fail "prior pending state"
    printf 'previous=0001 candidate=0002\n' > "$PREFIX/in-progress"
    sync "$PREFIX"
    systemctl stop "$SERVICE"
    atomic_link 0002
    systemctl restart "$SERVICE" || true
    if health; then fail "signed but invalid service unexpectedly healthy"; fi
    rollback
    test "$(readlink "$PREFIX/active")" = releases/0001
    echo "B1B2B_VM_FAILED_SIGNED_UPGRADE_ROLLBACK_PASS"
    ;;
  crash)
    test ! -e "$PREFIX/in-progress" || fail "prior pending state"
    test "$(readlink "$PREFIX/active")" = releases/0001
    printf 'previous=0001 candidate=0002\n' > "$PREFIX/in-progress"
    sync "$PREFIX"
    systemctl stop "$SERVICE"
    atomic_link 0002
    echo "B1B2B_VM_INJECT_SIGKILL_AFTER_ATOMIC_ACTIVATION"
    kill -9 "$$"
    fail "SIGKILL failed"
    ;;
  recover)
    test -f "$PREFIX/in-progress" || fail "no interrupted operation"
    test "$(readlink "$PREFIX/active")" = releases/0002 \
      || fail "unexpected active version while recovering"
    rollback
    test ! -e "$PREFIX/in-progress"
    echo "B1B2B_VM_RESTART_RECOVERY_PASS"
    ;;
  cleanup)
    systemctl stop "$SERVICE" || true
    rm -f -- "$UNIT"
    systemctl daemon-reload
    rm -rf -- "$PREFIX" "$FIXTURE" /var/lib/akula-b1b2b-lab-state \
      /var/lib/private/akula-b1b2b-lab-state
    rm -f -- "$CREDENTIAL"
    echo "B1B2B_VM_CLEANUP_PASS"
    ;;
  *) fail "unknown action" ;;
esac
