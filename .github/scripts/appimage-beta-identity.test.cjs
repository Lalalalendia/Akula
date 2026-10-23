'use strict';

// RELEASE-APPIMAGE-01 — static fail-closed identity and publication boundary.
// Deliberately verifies the *explicit* packager config, never Cargo.toml's
// inherited Aquilum metadata, and does not use any release-publishing secrets.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const file = (name) => readFileSync(path.join(root, name), 'utf8');

test('AppImage has an independent explicit Akula packager identity', () => {
  const cfg = JSON.parse(file('aquilum-app/native/packaging/akula-linux.json'));
  assert.equal(cfg.productName, 'Akula');
  assert.equal(cfg.version, '0.3.9');
  assert.equal(cfg.identifier, 'io.github.lalalendia.akula');
  assert.equal(cfg.homepage, 'https://github.com/Lalalalendia/Akula');
  assert.deepEqual(cfg.binaries, [{ path: 'akula-app', main: true }]);
  assert.equal(cfg.binariesDir, '../../../.artifacts/cargo-native/release');
  assert.ok(cfg.icons.every((v) => v.startsWith('../icons/')));
  assert.equal(Object.hasOwn(cfg, 'nsis'), false);
  assert.equal(Object.hasOwn(cfg, 'macos'), false);
  assert.match(cfg.appimage.linuxdeployPlugins.gstreamer, /\/[a-f0-9]{40}\/linuxdeploy-plugin-gstreamer.sh$/);
  assert.doesNotMatch(JSON.stringify(cfg), /com\.dmitriy|Freaction|Aquilum\.desktop|aquilum-app/);
});

test('AppImage remains unsigned and runs only through the required PR route', () => {
  const w = file('.github/workflows/appimage-beta.yml');
  const core = file('.github/workflows/app-checks.yml');
  const shared = file('.github/workflows/ci-shared-manifest-labs.yml');
  const route = require('./ci-shared-manifest-route.cjs');
  assert.match(w, /^name: Akula AppImage beta candidate$/m);
  assert.match(w, /^  workflow_call:$/m);
  assert.doesNotMatch(w, /^  (push|pull_request|release|workflow_dispatch|schedule):$/m);
  for (const owned of ['.github/workflows/appimage-beta.yml', '.github/scripts/appimage-beta-identity.test.cjs', 'aquilum-app/native/packaging/akula-linux.json'])
    assert.equal(route.ownedRoutes([owned]).owned_appimage, true, owned + ' must activate the required suite');
  assert.match(core, /owned_appimage: \$\{\{ needs\.classify\.outputs\.owned_appimage == 'true' \}\}/);
  assert.match(shared, /route-appimage-beta:\n    name: Impacted shared manifest \/ appimage-beta\n    if: \$\{\{ inputs\.lock \|\| inputs\.owned_appimage \}\}/);
  assert.match(w, /cargo build --release --locked -p aquilum-native --features installed/);
  assert.match(w, /cargo install cargo-packager --version 0\.11\.8 --locked/);
  assert.match(w, /--config native\/packaging\/akula-linux\.json/);
  assert.match(w, /--formats appimage/);
  assert.match(w, /actions\/upload-artifact@v7/);
  assert.doesNotMatch(w, /gh release|contents: write|TAURI_SIGNING|CARGO_PACKAGER_SIGN|sync-installed-host|macos-latest/);
});

test('OS-level AppImage acceptance preserves original Aquilum witnesses', () => {
  const s = file('scripts/appimage-beta-smoke.sh');
  assert.match(s, /--appimage-extract/);
  assert.match(s, /\^akula-app_\[0-9\]/, 'strictly validate observed cargo-packager file name');
  assert.match(s, /APPIMAGE_EXTRACT_AND_RUN=1/);
  assert.match(s, /wmctrl -c Akula/);
  assert.match(s, /AKULA_APPIMAGE_GUI_PASS/);
  assert.match(s, /AKULA_APPIMAGE_COEXIST_PASS/);
  assert.match(s, /sha256sum/);
  assert.match(s, /RUNNER_ENVIRONMENT:-/);
  // Catch the false green from docker run without -i: the heredoc silently
  // reads as EOF in the container. Marker checks on the *host* are required.
  assert.match(s, /docker run --rm -i/);
  assert.match(s, /\| tee "\$container_log"/);
  assert.match(s, /grep -Fxq 'AKULA_APPIMAGE_GUI_PASS[^']*' "\$container_log"/);
  assert.match(s, /grep -Fxq 'AKULA_APPIMAGE_COEXIST_PASS[^']*' "\$container_log"/);
  assert.match(s, /AKULA_APPIMAGE_HOST_GATE_PASS/);
  assert.doesNotMatch(s, /rm -rf "\$HOME"|systemctl|sudo/);
  const pkg = file('aquilum-app/native/packaging/akula-linux-nfpm.yaml');
  assert.match(pkg, /^name: akula$/m);
  const updater = file('aquilum-app/native/src/updater/release.rs');
  // The owner key may be pinned before release; pin its entire value so
  // a spoofed Minisign comment or a different publisher is not accepted.
  const pinned = updater.match(/pub\(super\) const PUBLIC_KEY: &str = "([^"]+)";/);
  assert.ok(pinned, 'Akula public release root must be explicitly pinned');
  assert.equal(pinned[1], 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ3QTY2QjZENDA0NkY2QzUKUldURjlrWkFiV3VtMXh2SFFsdEljaUw0T0pUakVmd1krZG4wVmJ1REVmaEx1eWcrMFIzTjQzMi8K', 'unexpected Akula publisher key');
  assert.match(updater, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*cfg!\(all\(target_os = "windows", feature = "installed"\)\)\s*\}/);
});
