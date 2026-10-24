# S3+P2P: real MinIO-origin browser test (manual Docker runner)

This isolated lab checks that the original 8 MiB bytes really come from an
S3-compatible server, while subsequent readers can use WebRTC instead.

- `tests/setup_minio_browser.py`: creates one throwaway test bucket with one
  synthetic immutable object in a **loopback-only MinIO** and enables anonymous
  read-only `GetObject`. It explicitly refuses non-loopback S3 endpoints.
- `tests/minio_tls_gateway.py`: read-only narrow reverse proxy to **that exact
  object**, terminated with a test-only certificate, so Chromium can request it
  via HTTPS. The server forwards GET/HEAD/Range to MinIO. The browser client
  never receives the MinIO upload credentials.
- `tests/test_minio_browser_webrtc.cjs`: actual Playwright Chromium
  `RTCPeerConnection` / `RTCDataChannel`; first reader obtains all checked
  bytes over HTTPS from MinIO, second receives all checked bytes from the first
  via P2P (MinIO `origin_bytes` unchanged), and third downloads from MinIO
  when upload permission is withdrawn.
- `tests/run_minio_browser_local.sh`: starts a disposable loopback-only MinIO,
  creates a synthetic public-read object, starts a test-only TLS gateway and
  tracker, then runs the Chromium end-to-end cases including malicious chunk
  rejection. It does not require GitHub workflow permissions.

## Manual run on a trusted Linux machine with Docker

Install Python dependencies into a dedicated virtual environment and use a
local copy of Playwright (no global browser configuration):

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r requirements-dev.txt "boto3>=1.34,<2"
npm install --no-save playwright@1.56.1
npx playwright install --with-deps chromium
bash tests/run_minio_browser_local.sh
```

The runner refuses missing Docker, occupied loopback ports, or a checkout
without the MinIO test files. It uses a private temporary working directory
and throwaway MinIO bucket, with only synthetic bytes. The entire setup is
bound to localhost; no production AWS credentials are used. On test failure,
the temporary test logs remain in the printed directory for diagnosis.
The browser test rejects a corrupt 256 KiB peer chunk and checks the exact
MinIO origin-byte deltas. A passing result proves only localhost
MinIO+WebRTC interoperability, not WAN.

**GitHub status:** The branch deliberately has no dedicated GitHub Actions
workflow. An earlier attempt to create it was blocked by tool security checks;
this manual script is a standalone permitted runner, not an automated CI PASS.

## Hard limitations

The MinIO container and browser instances are on **one local Linux host**. HTTPS uses
an **ephemeral, self-signed CI certificate** accepted only in the Playwright
browser context; not suitable for production. This is real S3-compatible
object storage but not AWS S3, not separate Internet providers, not two real
machines, not a NAT/TURN test and not a measurement of real S3 charges.

For real two-device acceptance: deploy a trusted HTTPS/WSS tracker and a
public-readable, immutable S3 URL; configure a trusted manifest (preferably
signed by the publisher). Launch separate browser clients on independent
networks and record peer ICE selected candidate type, bytes downloaded from
S3, direct WebRTC and relay, p50/p95 latency, and bandwidth/CPU caps. Avoid
opening the research tracker on the public Internet without rate limiting and
abuse controls.