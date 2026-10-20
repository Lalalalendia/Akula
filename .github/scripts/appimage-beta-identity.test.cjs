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

test('AppImage workflow is a pull-request-only disposable unsigned candidate', () => {
  const w = file('.github/workflows/appimage-beta.yml');
  assert.match(w, /^name: Akula AppImage beta candidate$/m);
  assert.match(w, /^  pull_request:$/m);
  assert.doesNotMatch(w, /^  (push|release|workflow_dispatch|schedule):$/m);
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
  assert.match(s, /APPIMAGE_EXTRACT_AND_RUN=1/);
  assert.match(s, /wmctrl -c Akula/);
  assert.match(s, /AKULA_APPIMAGE_GUI_PASS/);
  assert.match(s, /AKULA_APPIMAGE_COEXIST_PASS/);
  assert.match(s, /sha256sum/);
  assert.match(s, /RUNNER_ENVIRONMENT:-/);
  assert.match(s, /docker run --rm/);
  assert.doesNotMatch(s, /rm -rf "\$HOME"|systemctl|sudo/);
  const pkg = file('aquilum-app/native/packaging/akula-linux-nfpm.yaml');
  assert.match(pkg, /^name: akula$/m);
  const updater = file('aquilum-app/native/src/updater/release.rs');
  assert.match(updater, /pub\(super\) const PUBLIC_KEY: &str = "";/);
  assert.match(updater, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*false\s*\}/);
});
