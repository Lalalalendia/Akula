# One-PC synthetic fixtures

Run from the repository root:

```powershell
python scripts/one-pc/generate_fixtures.py
```

The generator creates only synthetic data in the ignored `.artifacts/one-pc-fixtures/` directory. It does not read personal files, delete files, or replace existing fixtures with different contents. Re-running it verifies existing outputs and completes any missing files after an interrupted run.

The fixture set includes an empty `empty-vault/` directory, 1k/10k/50k-note catalogs, 30k/100k-line Markdown files, 1/10/100 small valid PNG images, Unicode paths, same-name import conflicts, and malformed/truncated files. `SHA256SUMS.tsv` records the path, byte length, and SHA-256 for every generated test fixture (the manifest and summary describe themselves separately).

## Two isolated local process profiles

Prepare separate app-data directories and synthetic vault folders with:

```powershell
python scripts/one-pc/prepare_instances.py
```

The output is `.artifacts/one-pc-instances/A` and `B`. The desktop binary accepts explicit `--data-dir` and `--vault` arguments, so each process can use its own local state. The loopback addresses in `profile.json` are reserved hints, not active listeners.

The ordinary Windows P2P identity implementation uses one fixed Credential Manager target per logon session; `--data-dir` alone does not create an independent device key. The preparation script does not touch Credential Manager or save private keys. It records only a random test-target namespace in `p2p-test-targets.json`.

After building the native binary, launch one profile from each of two PowerShell windows without closing an already running Akula process:

```powershell
./scripts/one-pc/run_instance.ps1 -Instance A
./scripts/one-pc/run_instance.ps1 -Instance B
```

The launcher requires the profile paths to remain under `.artifacts/one-pc-instances/` and passes explicit `--data-dir` and `--vault` arguments. Running two UI processes proves local app-data separation only; it does not prove distinct P2P identities or two-device acceptance.

### Distinct P2P identities on this PC

Use the dedicated debug-only harness to give each process a separate, per-run Credential Manager target while keeping the ordinary identity target untouched:

```powershell
./scripts/one-pc/run_p2p_smoke.ps1
```

The harness validates its target namespace, tests that production targets are rejected, builds a separate debug output directory, creates two random private identities in Windows Credential Manager, launches both prepared profiles with their own target, checks distinct public EndpointIds and app data, closes both windows, deletes both temporary credentials, and verifies that they are absent. Harness credentials use Windows logon-session persistence and are deleted at the end; ordinary and installed builds retain the existing machine-persistent per-user target. Private key bytes never enter manifests or logs. The core feature is named `p2p-device-identity-test-harness`, requires an explicit test target, and fails compilation if enabled in a release build.

The loopback address values in the profiles are distinct configuration hints; main does not open P2P listeners. This smoke proves local identity/profile/process setup only. It does not exercise a transfer, a real network, a second physical device, or the external acceptance required by the P2P integration PRs. Logs and public EndpointIds are retained under `.artifacts/one-pc-p2p-smoke/`.

If a window remains open and cleanup is reported as pending, close all Akula windows, then use the run ID from `summary.json`:

```powershell
./scripts/one-pc/cleanup_p2p_smoke.ps1 -RunId 20261024-000000000
```

Cleanup refuses to run while any `aquilum-native` process is present and only accepts the per-run test namespace from that smoke's summary.

## Local quality runs

Run a quick local lane first: formatting checks for changed Rust files, the Core test suite, a fresh native debug build, and the two-process GUI smoke:

```powershell
./scripts/one-pc/run_checks.ps1 -Step quick
```

Run the complete local lane with `-Step all`, or choose `fmt`, `fmt-changed`, `clippy`, `core`, `sync`, `native`, `build-native`, `test`, `audit`, or `gui`. `-TimeoutMinutes` bounds each Cargo step. Logs, resource samples, exact starting/ending HEAD, lockfile hashes, dirty-state flag, run ID and a JSON summary are retained under `.artifacts/one-pc-checks/` even when a check fails. The summary explicitly says this is not the GitHub Main PR gate.

Cargo runs use `CARGO_INCREMENTAL=0` so repeated local checks do not accumulate large incremental caches on the development PC. This can make recompilation slower after source changes.

On Windows, the runner restricts only the disposable `core-temp`, `sync-temp`, `native-temp`, or `test-temp` directory to the current user, SYSTEM and Administrators before running tests. This prevents inherited workspace ACLs from making File Gate's intentional owner/DACL checks fail. Cargo output, application data and user folders are not re-permissioned.

The `fmt` step checks the full Rust workspace. The `fmt-changed` step uses each file's Cargo edition and skips child modules. It checks whole untracked Rust files and the changed lines of tracked files; when rustfmt reports drift only outside those changed lines, the runner records it as inherited instead of reformatting unrelated code. The quick lane rebuilds `aquilum-native` before opening the two local profiles. A standalone `gui` run refuses to launch a binary older than its current Core/Native sources; run `build-native` first.

The app-checks workflow uses Rust stable and selects test groups from changed paths; its required Main PR gate and RustSec checks remain authoritative. `native-check.yml` has a manual `workflow_dispatch` for diagnostic Windows/Linux runs. Before dispatch, inspect its current inputs, target exact commit, and existing in-progress runs. The plan's read-only diagnostics and external acceptance boundaries still apply; never use the held `release.yml` for testing.

PowerShell example for a tests-only diagnostic run after selecting a target that exists on GitHub:

```powershell
gh workflow run native-check.yml --repo Lalalalendia/Akula --ref main `
  --field ref=main --field tests-only=true `
  --field 'platforms=["windows-latest","ubuntu-22.04"]'
```

This dispatch example is documented, not run by the local script. A workflow run checks only its recorded `head_sha`; it does not replace local evidence, the required Main PR gate or external-device acceptance. Results are written locally here for later review; this script does not update Notion.
