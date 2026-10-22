# Akula P2P Internet Test Candidate — NOT a final Sync release

**Source snapshot:** a test-only branch combining [manual pairing UI PR #609](https://github.com/Lalalalendia/Akula/pull/609) with [P2P/Staya bridge PR #633](https://github.com/Lalalalendia/Akula/pull/633), not yet merged into main. Do not use it as a signed stable Akula release.

**Contents:** unsigned Akula 0.3.11 NSIS installer (local editor + manual peer-pairing UI, but NO working GUI Send button); two separate diagnostic executables from the same source checkout; SHA256SUMS and CI provenance.

## You do NOT need a shared local network

The two Windows computers can be in completely different places and behind ordinary routers, connected to the Internet. There are two options:

1. **Internet via Iroh N0** (explicit third-party infrastructure opt-in): `internet-listen` and `internet-connect`. Both devices must independently confirm each other's full 64-character public EndpointIds. N0 DNS/Pkarr services and relay fallback will be used only in the explicit internet modes, and may learn IPs, packet timings and sizes. QUIC payload stays end-to-end encrypted.
2. **Direct IP** (private/VPN/overlay or port-forwarded/public UDP): `listen` and `connect`. No N0 discovery or relay. Both hosts may be in different networks if they can route packets to the listener's UDP address. A private Tailscale/WireGuard overlay can supply reachability without the hosts sharing physical LAN.

## Minimal proof on Windows A and Windows B

Before testing, use disposable data and accounts. These unsigned executables are review-only GitHub Actions artifacts, not a signed stable publisher release. Verify `SHA256SUMS.txt` before executing.

On **each** Windows machine, once only for a Windows account with no prior enrollment:

```powershell
.\akula-p2p-identity.exe --init
.\akula-p2p-identity.exe --id
```

If already initialized, skip `--init`; do not regenerate a key. Exchange full public IDs out of band; on each machine independently compare/retype the OTHER machine's ID:

```powershell
.\akula-p2p-identity.exe --peer-confirm OTHER_ENDPOINT_ID
```

Machine **A**:

```powershell
.\akula-p2p-lan.exe internet-listen OTHER_ENDPOINT_ID
```

Machine **B**, over any other internet connection:

```powershell
.\akula-p2p-lan.exe internet-connect OTHER_ENDPOINT_ID
```

Expected: both tools print `OS_PINNED_QUIC_OK=true ROUTE_MODE=INTERNET_N0 FILE_TRANSFER=false SYNC=false`. The listener only waits five minutes for an incoming peer; rerun if the session expires. Actual success depends on the two devices, n0 reachability, DNS and outbound traffic rules.

For **direct** link (e.g. your own private WireGuard network), on A use `listen OTHER_ID`, find the printed UDP port; B uses `connect OTHER_ID REACHABLE_A_IP:UDP_PORT`. No shared physical LAN is required if the network permits direct UDP.

## Tested functionality vs outstanding work

- The installer is built and exercised by GitHub-hosted Windows UIA/edit/restart/uninstall smoke **only if this exact branch Actions run succeeds**. A successful standalone diagnostic does not imply the GUI can transfer a file.
- The Windows CLI is compiled by CI. Its direct/online success against two real devices is **not** tested automatically.
- This candidate **does not yet** perform Core→Staya production sender grant, persist file-transfer results in the GUI, or apply remote data through a verified File Gate/HistoryService/native ACK.
- Network peer pin is **not** permission to read a Vault. None of the provided commands transmit user files.
- No protected release key, signature, update channel or GitHub Releases publishing is involved.
- Do not combine this unsigned test branch with real personal Vault data until actual end-to-end acceptance and security review.
