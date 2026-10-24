#!/usr/bin/env bash
# Distinct Docker network namespaces connected over an explicit bridge.
# The test host is the only source/tracker; no public WAN/NAT claims.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
mkdir -p test-results
RUN_SUFFIX="${GITHUB_RUN_ID:-manual}-$$"
NETWORK="akula-p2p-net-${RUN_SUFFIX}"
CHROMIUM_CONTAINER="akula-p2p-a-${RUN_SUFFIX}"
FIREFOX_CONTAINER="akula-p2p-b-${RUN_SUFFIX}"
IMAGE="mcr.microsoft.com/playwright:v1.56.1-noble"

cleanup() {
  local exit_code=$?
  set +e
  docker logs "$CHROMIUM_CONTAINER" > test-results/namespaced-chromium-container.log 2>&1
  docker logs "$FIREFOX_CONTAINER" > test-results/namespaced-firefox-container.log 2>&1
  docker rm -f "$CHROMIUM_CONTAINER" "$FIREFOX_CONTAINER" >/dev/null 2>&1
  docker network rm "$NETWORK" >/dev/null 2>&1
  exit "$exit_code"
}
trap cleanup EXIT

docker network create "$NETWORK" >/dev/null

for item in "chromium:$CHROMIUM_CONTAINER:9331" "firefox:$FIREFOX_CONTAINER:9332"; do
  IFS=: read -r engine name hostport <<< "$item"
  docker run --detach --rm \
    --network "$NETWORK" \
    --name "$name" \
    --shm-size=1g \
    --add-host=host.docker.internal:host-gateway \
    --publish "127.0.0.1:${hostport}:3000" \
    --volume "$PWD:/work:ro" \
    --workdir /work \
    "$IMAGE" \
    bash -lc 'node tests/proxy_origin.cjs & exec ./node_modules/.bin/playwright run-server --host 0.0.0.0 --port 3000' >/dev/null
  echo "Started $engine in isolated namespace ($name)"
done

A_PID=$(docker inspect --format '{{.State.Pid}}' "$CHROMIUM_CONTAINER")
B_PID=$(docker inspect --format '{{.State.Pid}}' "$FIREFOX_CONTAINER")
A_NS=$(readlink "/proc/$A_PID/ns/net")
B_NS=$(readlink "/proc/$B_PID/ns/net")
A_IP=$(docker inspect --format "{{with index .NetworkSettings.Networks \"${NETWORK}\"}}{{.IPAddress}}{{end}}" "$CHROMIUM_CONTAINER")
B_IP=$(docker inspect --format "{{with index .NetworkSettings.Networks \"${NETWORK}\"}}{{.IPAddress}}{{end}}" "$FIREFOX_CONTAINER")
if [[ "$A_NS" == "$B_NS" || "$A_IP" == "$B_IP" || -z "$A_IP" || -z "$B_IP" ]]; then
  echo "ERROR: browsers share a network namespace/IP address" >&2
  exit 1
fi
printf 'same_physical_host=yes\nnamespace_A=%s\nnamespace_B=%s\nip_A=%s\nip_B=%s\nbridge=%s\n' "$A_NS" "$B_NS" "$A_IP" "$B_IP" "$NETWORK" > test-results/netns-evidence.txt
cat test-results/netns-evidence.txt

for port in 9331 9332; do
  ready=0
  for _ in $(seq 1 60); do
    if node -e 'const net=require("node:net");const s=net.connect(Number(process.argv[1]),"127.0.0.1");s.setTimeout(750);s.once("connect",()=>{s.destroy();process.exit(0)});s.once("error",()=>process.exit(1));s.once("timeout",()=>process.exit(1))' "$port"; then
      ready=1
      break
    fi
    sleep 1
  done
  if [[ "$ready" != 1 ]]; then
    echo "ERROR: remote Playwright port $port not ready" >&2
    exit 1
  fi
done

# A browser's localhost is its own TCP proxy, NOT the runner's localhost.
for container in "$CHROMIUM_CONTAINER" "$FIREFOX_CONTAINER"; do
  docker exec "$container" node -e 'fetch("http://127.0.0.1:8765/manifest.json").then(r=>{if(!r.ok)process.exit(1);return r.json()}).then(x=>{if(x.size!==8388608)process.exit(1)}).catch(()=>process.exit(1))'
done

LAB_CHROMIUM_WS='ws://127.0.0.1:9331/' LAB_FIREFOX_WS='ws://127.0.0.1:9332/' \
  timeout 240 node tests/test_namespaced_webrtc.cjs