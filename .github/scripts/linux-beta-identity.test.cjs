'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../..');
const file = (p) => readFileSync(path.join(root,p), 'utf8');

test('independent Linux runtime data, autostart, and migration scope', () => {
  const w=file('aquilum-app/native/src/workspace.rs');
  assert.match(w, /#\[cfg\(any\(target_os = "windows", target_os = "linux"\)\)\]\s+pub const APP_ID: &str = "io.github.lalalendia.akula";/);
  assert.match(w, /#\[cfg\(not\(any\(target_os = "windows", target_os = "linux"\)\)\)\]\s+if data_dir\.ends_with\(APP_ID\)/);
  assert.match(file('aquilum-app/native/src/autostart.rs'), /#\[cfg\(any\(target_os = "windows", target_os = "linux"\)\)\]\s+const NAME: &str = "Akula";/);
  assert.match(file('aquilum-app/native/src/app/lifecycle.rs'), /any\(target_os = "windows", target_os = "linux"\)/);
});
test('package identity and desktop files never own Aquilum paths', () => {
  const p=file('aquilum-app/native/packaging/akula-linux-nfpm.yaml');
  const d=file('aquilum-app/native/packaging/Akula.desktop');
  assert.match(p, /^name: akula$/m);
  assert.match(p, /^vendor: Akula$/m);
  assert.match(p, /src: packaging\/stage\/akula-app\s+dst: \/usr\/bin\/akula-app/);
  assert.match(p, /src: packaging\/Akula.desktop\s+dst: \/usr\/share\/applications\/Akula.desktop/);
  assert.doesNotMatch(p, /Freaction|Dmitriy|Aquilum|aquilum-app|com.dmitriy/);
  assert.equal((p.match(/- xdg-desktop-portal/g) || []).length, 2, 'both DEB and RPM must depend on portal');
  assert.match(d, /^Name=Akula$/m);
  assert.match(d, /^Exec=akula-app$/m);
  assert.match(d, /^Icon=akula-app$/m);
  assert.match(file('aquilum-app/native/packaging/nfpm.yaml'), /^name: aquilum$/m);
});
test('CI is not a release and updater stays fail closed', () => {
  const ci=file('.github/workflows/linux-beta.yml'),smoke=file('scripts/linux-beta-smoke.sh');
  assert.match(ci, /^name: Akula Linux beta candidate$/m);
  assert.match(ci, /cargo build --release --locked -p aquilum-native --features installed/);
  assert.match(ci, /akula-linux-nfpm.yaml/);
  assert.doesNotMatch(ci, /sync-installed-host|gh release create|docker push|TAURI_SIGNING_PRIVATE_KEY/);
  assert.match(ci, /actions\/upload-artifact@v7/);
  assert.match(smoke, /AKULA_LINUX_BETA_INSTALL_PASS/);
  assert.match(smoke, /AKULA_LINUX_BETA_COEXIST_PASS/);
  // The old workflow was a false green: Docker stdin was disabled and
  // shell scripts never ran. Stdin AND verified child evidence are mandatory.
  assert.match(smoke, /docker run --rm -i -v[^\n]*ubuntu:22\.04[^\n]*<<'UBUNTU' \| tee "\$ubuntu_log"/);
  assert.match(smoke, /docker run --rm -i -v[^\n]*fedora:latest[^\n]*<<'FEDORA' \| tee "\$fedora_log"/);
  assert.match(smoke, /grep -Fxq 'AKULA_LINUX_BETA_INSTALL_PASS[^']*' "\$ubuntu_log"/);
  assert.match(smoke, /grep -Fxq 'AKULA_LINUX_BETA_COEXIST_PASS[^']*' "\$ubuntu_log"/);
  assert.match(smoke, /grep -Fxq 'AKULA_LINUX_BETA_COEXIST_PASS[^']*' "\$fedora_log"/);
  assert.match(smoke, /AKULA_LINUX_BETA_HOST_GATE_PASS/);
  assert.match(smoke, /dpkg-query -W -f='\\$\\{Status\\}' xdg-desktop-portal/);
  assert.match(smoke, /rpm -q xdg-desktop-portal/);
  const updater=file('aquilum-app/native/src/updater/release.rs');
  // The owner key may be pinned before release; pin its entire value so
  // a spoofed Minisign comment or a different publisher is not accepted.
  const pinned = updater.match(/pub\(super\) const PUBLIC_KEY: &str = "([^"]+)";/);
  assert.ok(pinned, 'Akula public release root must be explicitly pinned');
  assert.equal(pinned[1], 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ3QTY2QjZENDA0NkY2QzUKUldURjlrWkFiV3VtMXh2SFFsdEljaUw0T0pUakVmd1krZG4wVmJ1REVmaEx1eWcrMFIzTjQzMi8K', 'unexpected Akula publisher key');
  assert.match(updater, /pub\(super\) const fn channel_ready\(\) -> bool \{\s*cfg!\(all\(target_os = "windows", feature = "installed"\)\)\s*\}/);
});
