#!/usr/bin/env bash
set -euo pipefail

if [[ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" || "${RUNNER_OS:-}" != "Linux" ]]; then
  echo "Refusing AppImage execution outside an ephemeral GitHub-hosted Linux runner" >&2
  exit 1
fi
shopt -s nullglob
images=("$PWD"/dist/*.AppImage)
[[ "${#images[@]}" == 1 ]] || { echo "Expected exactly one AppImage candidate" >&2; exit 1; }
image="${images[0]}"
[[ "$(basename "$image")" =~ ^akula-app_[0-9]+\.[0-9]+\.[0-9]+_x86_64\.AppImage$ ]] || {
  echo "Unexpected product name for AppImage candidate: $(basename "$image")" >&2; exit 1
}
test -s "$image"
chmod u+x "$image"

# Inspect actual packaged contents, not just an external filename or YAML.
extract="$(mktemp -d)"
trap 'rm -rf "$extract"' EXIT
(
  cd "$extract"
  "$image" --appimage-extract > /dev/null
)
dir="$extract/squashfs-root"
test -x "$dir/AppRun"
test -x "$dir/usr/bin/akula-app"
if find "$dir" \( -name 'Aquilum.desktop' -o -name 'aquilum-app' \) -print | grep -q .; then
  echo "AppImage embeds upstream Aquilum package identity" >&2; exit 1
fi
mapfile -t desktop_files < <(find "$dir" -type f -name '*.desktop')
(( "${#desktop_files[@]}" > 0 )) || { echo "AppImage contains no desktop integration" >&2; exit 1; }
accepted=0
for desktop in "${desktop_files[@]}"; do
  if grep -qx 'Name=Akula' "$desktop" && grep -qE '^Exec=(akula-app|AppRun)([[:space:]]|$)' "$desktop"; then
    accepted=1
    break
  fi
done
[[ "$accepted" == 1 ]] || {
  echo "Extracted AppImage desktop identity does not identify Akula" >&2; exit 1
}
echo 'AKULA_APPIMAGE_STRUCTURE_PASS apprun=1 own_bin=1 own_desktop=1 upstream_paths=0'

# The AppImage is executed in an isolated Docker root FS with isolated
# HOME/XDG_DATA_HOME and only synthetic Aquilum witness data.
docker run --rm -v "$PWD/dist:/dist:ro" ubuntu:22.04 bash -s -- "/dist/$(basename "$image")" <<'UBUNTU'
set -euo pipefail
image="$1"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  ca-certificates xvfb openbox wmctrl file libglib2.0-0 libfontconfig1 libfreetype6 \
  libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
  libxkbcommon0 libxkbcommon-x11-0 libx11-6 libx11-xcb1 \
  libxcursor1 libxrandr2 libxi6 libxcb1 libwayland-client0
mkdir -p /usr/share/applications /tmp/user/.local/share/com.dmitriy.aquilum-app
printf 'upstream binary witness' > /usr/bin/aquilum-app
printf 'upstream desktop witness' > /usr/share/applications/Aquilum.desktop
printf 'upstream appdata witness' > /tmp/user/.local/share/com.dmitriy.aquilum-app/keep
witness_before="$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)"
export HOME=/tmp/user XDG_DATA_HOME=/tmp/user/.local/share DISPLAY=:99
export APPIMAGE_EXTRACT_AND_RUN=1
mkdir -p /tmp/vault
printf '# AppImage acceptance\n\nOriginal synthetic Markdown\n' > /tmp/vault/First.md
original="$(sha256sum /tmp/vault/First.md)"

Xvfb :99 -screen 0 1024x768x24 > /tmp/akula-xvfb.log 2>&1 &
xvfb_pid=$!
sleep 2
openbox > /tmp/akula-wm.log 2>&1 &
wm_pid=$!
app_pid=""
cleanup() {
  if [[ -n "$app_pid" ]]; then kill "$app_pid" 2>/dev/null || true; fi
  kill "$wm_pid" "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT

"$image" --vault /tmp/vault --open /tmp/vault/First.md > /tmp/akula-appimage.log 2>&1 &
app_pid=$!
ready=0
for attempt in $(seq 1 55); do
  kill -0 "$app_pid" 2>/dev/null || {
    echo "AppImage exited before showing Akula window" >&2
    tail -45 /tmp/akula-appimage.log >&2
    exit 1
  }
  if wmctrl -l | grep -q 'Akula'; then ready=1; break; fi
  sleep 1
done
[[ "$ready" == 1 ]] || { echo "AppImage did not create Akula GUI" >&2; tail -45 /tmp/akula-appimage.log; exit 1; }
wmctrl -c Akula
closed=0
for attempt in $(seq 1 45); do
  if ! kill -0 "$app_pid" 2>/dev/null; then closed=1; break; fi
  sleep 0.5
done
[[ "$closed" == 1 ]] || { echo "AppImage GUI refused to exit" >&2; exit 1; }
wait "$app_pid"
app_pid=""
test -f "$XDG_DATA_HOME/io.github.lalalendia.akula/window-state.json"
[[ "$(sha256sum /tmp/vault/First.md)" == "$original" ]] || {
  echo "Synthetic Markdown changed during AppImage GUI acceptance" >&2; exit 1
}
echo 'AKULA_APPIMAGE_GUI_PASS app=Akula isolated_xdg=1 original_note_intact=1'
[[ "$(sha256sum /usr/bin/aquilum-app /usr/share/applications/Aquilum.desktop /tmp/user/.local/share/com.dmitriy.aquilum-app/keep)" == "$witness_before" ]] || {
  echo "Aquilum witness changed during Akula AppImage acceptance" >&2; exit 1
}
echo 'AKULA_APPIMAGE_COEXIST_PASS upstream_executable=unchanged upstream_desktop=unchanged upstream_appdata=unchanged'
UBUNTU
