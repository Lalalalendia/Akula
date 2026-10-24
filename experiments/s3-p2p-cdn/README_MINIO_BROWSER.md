# S3+P2P: real MinIO-origin browser test (CI only)

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
- `.github/workflows/s3-p2p-minio-browser.yml`: runs the test on a GitHub
  Linux runner, reports counters and protocol logs. Distinct from the earlier
  browser-only HTTP simulator and separate Docker namespaces scenarios.

## Hard limitations

The MinIO container and both browsers are on **one GitHub runner**. HTTPS uses
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
