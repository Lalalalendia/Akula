#!/usr/bin/env bash
set -euo pipefail
if [[ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" || "${RUNNER_OS:-}" != "Linux" ]]; then
  echo "Refusing to install Linux candidates outside ephemeral GitHub-hosted runner" >&2
  exit 1
fi
version="${1:?Version required}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 1
deb="$PWD/dist/Akula_${version}_amd64.deb"
rpm="$PWD/dist/Akula-${version}-1.x86_64.rpm"
[[ -f "$deb" && -f "$rpm" ]] || exit 1
[[ "$(dpkg-deb -f "$deb" Package)" == "akula" ]]
[[ "$(rpm -qp --qf '%{NAME}' "$rpm")" == "akula" ]]
deblist="$(dpkg-deb -c "$deb")"
rpmlist="$(rpm -qpl "$rpm")"
for entry in usr/bin/akula-app usr/share/applications/Akula.desktop; do
  grep -Fq "$entry" <<< "$deblist"
  grep -Fq "/$entry" <<< "$rpmlist"
done
! grep -Eq '/aquilum-app|/Aquilum.desktop' <<< "$deblist $rpmlist"

# A Docker run without -i silently detaches stdin: bash -s immediately exits
# without running the heredoc. This MUST be detected as an acceptance failure.
ubuntu_log="$(mktemp)"
fedora_log="$(mktemp)"
trap 'rm -f "$ubuntu_log" "$fedora_log"' EXIT
# Never install on host. Container data is disposable. Pipefail preserves errors.
docker run --rm -i -v "$PWD/dist:/dist:ro" ubuntu:22.04 bash -s -- "/dist/Akula_${version}_amd64.deb" <<'UBUNTU' | tee "$ubuntu_log"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
mkdir -p /usr/share/applications /tmp/user/.local/share/com.dmitriy.aquilum-app
printf 'upstream binary' > /usr/bin/aquilum-app
printf 'upstream desktop' > /usr/share/applications/Aquilum.desktop
printf 'upstream state' > /tmp/user/.local/share/com.dmitriy.aquilum-app/keep
before="$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)"
apt-get update -qq
apt-get install -y -qq --no-install-recommends "$1" xvfb openbox wmctrl
dpkg-query -W -f='${Status}' xdg-desktop-portal | grep -Fxq 'install ok installed'
dpkg-query -W akula | grep -q '^akula'
test -x /usr/bin/akula-app
grep -q '^Exec=akula-app$' /usr/share/applications/Akula.desktop
export HOME=/tmp/user XDG_DATA_HOME=/tmp/user/.local/share DISPLAY=:99
mkdir -p /tmp/vault
printf '# Synthetic Akula Linux gate\n\noriginal\n' > /tmp/vault/First.md
hash="$(sha256sum /tmp/vault/First.md)"
Xvfb :99 -screen 0 1024x768x24 > /tmp/xvfb.log 2>&1 &
xpid=$!
sleep 2
openbox > /tmp/wm.log 2>&1 &
wpid=$!
trap 'kill "$wpid" "$xpid" 2>/dev/null || true' EXIT
/usr/bin/akula-app --vault /tmp/vault --open /tmp/vault/First.md > /tmp/app.log 2>&1 &
pid=$!
ready=0
for i in $(seq 1 45); do
  kill -0 "$pid" 2>/dev/null || { tail -25 /tmp/app.log; exit 1; }
  if wmctrl -l | grep -q 'Akula'; then ready=1; break; fi
  sleep 1
done
[[ "$ready" == 1 ]] || { tail -25 /tmp/app.log; exit 1; }
wmctrl -c Akula
closed=0
for i in $(seq 1 40); do
  if ! kill -0 "$pid" 2>/dev/null; then closed=1; break; fi
  sleep 0.5
done
[[ "$closed" == 1 ]]
wait "$pid"
test -f "$XDG_DATA_HOME/io.github.lalalendia.akula/window-state.json"
[[ "$(sha256sum /tmp/vault/First.md)" == "$hash" ]]
echo 'AKULA_LINUX_BETA_INSTALL_PASS app=Akula persisted=1 synthetic_unchanged=1'
dpkg -r akula >/dev/null
test ! -e /usr/bin/akula-app
test ! -e /usr/share/applications/Akula.desktop
test -f "$XDG_DATA_HOME/io.github.lalalendia.akula/window-state.json"
[[ "$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)" == "$before" ]]
echo 'AKULA_LINUX_BETA_COEXIST_PASS platform=ubuntu22 upstream_unchanged=1'
UBUNTU
# Exit status alone is insufficient. Require both child-generated markers.
grep -Fxq 'AKULA_LINUX_BETA_INSTALL_PASS app=Akula persisted=1 synthetic_unchanged=1' "$ubuntu_log"
grep -Fxq 'AKULA_LINUX_BETA_COEXIST_PASS platform=ubuntu22 upstream_unchanged=1' "$ubuntu_log"
docker run --rm -i -v "$PWD/dist:/dist:ro" fedora:latest bash -s -- "/dist/Akula-${version}-1.x86_64.rpm" <<'FEDORA' | tee "$fedora_log"
set -euo pipefail
mkdir -p /usr/share/applications
printf 'upstream binary' > /usr/bin/aquilum-app
printf 'upstream desktop' > /usr/share/applications/Aquilum.desktop
before="$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop)"
dnf install -y -q "$1"
rpm -q xdg-desktop-portal >/dev/null
rpm -q akula >/dev/null
test -x /usr/bin/akula-app
grep -q '^Exec=akula-app$' /usr/share/applications/Akula.desktop
dnf remove -y -q akula
test ! -e /usr/bin/akula-app
test ! -e /usr/share/applications/Akula.desktop
[[ "$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop)" == "$before" ]]
echo 'AKULA_LINUX_BETA_COEXIST_PASS platform=fedora upstream_unchanged=1'
FEDORA
grep -Fxq 'AKULA_LINUX_BETA_COEXIST_PASS platform=fedora upstream_unchanged=1' "$fedora_log"
echo 'AKULA_LINUX_BETA_HOST_GATE_PASS ubuntu_real_gui=1 fedora_real_package=1 child_output_required=1'
