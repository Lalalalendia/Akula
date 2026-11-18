'use strict';
const {test}=require('node:test');const assert=require('node:assert/strict');
const fs=require('node:fs');const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const paths={cargo:'aquilum-app/native/Cargo.toml',core:'aquilum-app/core/Cargo.toml',
  windows:'aquilum-app/native/packaging/akula-windows.json',lock:'aquilum-app/Cargo.lock',
  build:'aquilum-app/native/build.rs',verifier:'labs/video-lite/video18c02b04-native-pe-version.rs',
  workflow:'.github/workflows/akula-video18c02b05-real-native-version.yml',
  doc:'docs/release/video18c02b05-real-native-version.md'};
const read=k=>fs.readFileSync(path.join(root,paths[k]),'utf8');
function gate(a){
  assert.match(a.cargo,/\[package\]\s+name = "aquilum-native"\s+version = "0\.3\.21"/,
    'VIDEO18C02B05_HOLD: Cargo source candidate version mismatch');
  assert.ok(a.lock.includes('name = "aquilum-native"\nversion = "0.3.21"\n'),
    'VIDEO18C02B05_HOLD: locked native version mismatch');
  assert.match(a.core,/\\[package\\]\\s+name = "aquilum-core"\\s+version = "0\\.3\\.21"/,
    'VIDEO18C02B05_HOLD: core version differs from native');
  assert.ok(a.lock.includes('name = "aquilum-core"\\nversion = "0.3.21"\\n'),
    'VIDEO18C02B05_HOLD: core locked package version differs');
  assert.equal(JSON.parse(a.windows).version,'0.3.21',
    'VIDEO18C02B05_HOLD: Windows packager version differs');
  for(const s of ['CARGO_PKG_VERSION','win32_fixed_version_info(&package_version)'])
    assert.ok(a.build.includes(s),'VIDEO18C02B05_HOLD: native PE version resource bypass');
  for(const s of ['GetFileVersionInfoW','VerQueryValueW',
    'signed!=embedded || embedded<=installed','EXPECTED_HEAD_SHA'])
    assert.ok(a.verifier.includes(s),'VIDEO18C02B05_HOLD: version floor bypass');
  const head=a.workflow.split(/\r?\npermissions:/,1)[0];
  for(const p of [...Object.values(paths),'.github/scripts/video18c02b05-real-native-version.test.cjs'])
    assert.ok(head.includes("      - '"+p+"'"),'VIDEO18C02B05_HOLD: missing PR file trigger '+p);
  for(const s of ['cargo build --release --locked -p aquilum-native',
    'cargo metadata --locked --no-deps --format-version 1',
    'node scripts/akula-version.cjs check',
    'VIDEO18C02B04_NATIVE_PE_IDENTITY_PASS','VIDEO18C02B04_STRICT_VERSION_FLOOR_PASS',
    'VIDEO18C02B04_HOLD: signed label is not a genuine newer native PE version',
    '$global:LASTEXITCODE=0','VIDEO18C02B05_GENUINE_PE_RAISE_PASS',
    "cancel-in-progress: ${{ github.event_name == 'pull_request' }}"])
    assert.ok(a.workflow.includes(s),'VIDEO18C02B05_HOLD: missing native Windows acceptance '+s);
  assert.ok(!/workflow_dispatch:|contents:\s*write|gh release create/.test(a.workflow),
    'VIDEO18C02B05_HOLD: release action forbidden');
  for(const s of ['RELEASE HOLD','0.3.20','0.3.21','owner signing'])
    assert.ok(a.doc.includes(s),'VIDEO18C02B05_HOLD: boundary absent '+s);
}
test('Real new native source, locked version, PE and strict Windows floor',()=>{
  gate(Object.fromEntries(Object.keys(paths).map(k=>[k,read(k)])));
});
test('Negative mutations cannot bypass version, signing boundary or CI proof',()=>{
  const source=Object.fromEntries(Object.keys(paths).map(k=>[k,read(k)]));
  for(const [k,from,to] of [
    ['cargo','version = "0.3.21"','version = "0.3.20"'],
    ['core','version = "0.3.21"','version = "0.3.20"'],
    ['windows','"version": "0.3.21"','"version": "0.3.20"'],
    ['lock','name = "aquilum-core"\\nversion = "0.3.21"','name = "aquilum-core"\\nversion = "0.3.20"'],
    ['lock','name = "aquilum-native"\nversion = "0.3.21"','name = "aquilum-native"\nversion = "0.3.20"'],
    ['build','CARGO_PKG_VERSION','FAKE_VERSION'],
    ['verifier','signed!=embedded || embedded<=installed','signed==embedded && embedded>installed'],
    ['workflow','VIDEO18C02B05_GENUINE_PE_RAISE_PASS','FAKE_SUCCESS'],
    ['workflow','$global:LASTEXITCODE=0','$global:LASTEXITCODE=1'],
    ['workflow','cargo build --release --locked -p aquilum-native','echo fake build'],
    ['workflow',"cancel-in-progress: ${{ github.event_name == 'pull_request' }}","cancel-in-progress: true"]
  ]){
    const value=source[k].replaceAll(from,to);
    assert.notEqual(value,source[k],'VIDEO18C02B05_HOLD: source mutation absent '+k);
    assert.throws(()=>gate({...source,[k]:value}),/VIDEO18C02B05_HOLD/);
  }
});
