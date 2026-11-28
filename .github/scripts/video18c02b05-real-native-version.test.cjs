'use strict';
const {test}=require('node:test');const assert=require('node:assert/strict');
const fs=require('node:fs');const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const version=require('../../scripts/akula-version.cjs');
const paths={native:version.PATHS.native,core:version.PATHS.core,windows:version.PATHS.windows,
  videoLab:version.PATHS.videoLab,lock:version.PATHS.lock,version:'scripts/akula-version.cjs',
  build:'aquilum-app/native/build.rs',verifier:'labs/video-lite/video18c02b04-native-pe-version.rs',
  workflow:'.github/workflows/akula-video18c02b05-real-native-version.yml',
  nsis:'.github/workflows/akula-video17-nsis-installed-video.yml',
  doc:'docs/release/video18c02b05-real-native-version.md'};
const read=k=>fs.readFileSync(path.join(root,paths[k]),'utf8');
function gate(a){
  assert.doesNotThrow(()=>version.check(a),'VIDEO18C02B05_HOLD: Windows source version mirrors disagree');
  for(const token of [
    '$candidate=Join-Path $dist ("Akula_{0}_x64-setup.exe" -f $cfg.version)',
    '$setup=Join-Path $env:GITHUB_WORKSPACE ("dist/Akula_{0}_x64-setup.exe" -f $cfg.version)'
  ])assert.ok(a.nsis.includes(token),'VIDEO18C02B05_HOLD: NSIS installer filename version drift');
  assert.doesNotMatch(a.nsis,/Akula_0\\.3\\.20_x64-setup\\.exe/,
    'VIDEO18C02B05_HOLD: old installer version hardcoded');
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
    'gh api --paginate',
    "select(.draft == false)",
    '$_ -lt [version]$sourceVersion',
    '$sourceVersion $previousVersion $sourceVersion',
    '@($previousVersion,$previousVersion),@($previousVersion,$futureVersion),@($sourceVersion,$sourceVersion)',
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
  const current=version.check(source), future=version.nextPatch(current);
  for(const [k,from,to] of [
    ['native',`version = "${current}"`,`version = "${future}"`],
    ['core',`version = "${current}"`,`version = "${future}"`],
    ['windows',`"version": "${current}"`,`"version": "${future}"`],
    ['videoLab',`"version": "${current}"`,`"version": "${future}"`],
    ['nsis','$candidate=Join-Path $dist ("Akula_{0}_x64-setup.exe" -f $cfg.version)','$candidate=Join-Path $dist "Akula_0.3.20_x64-setup.exe"'],
    ['nsis','$setup=Join-Path $env:GITHUB_WORKSPACE ("dist/Akula_{0}_x64-setup.exe" -f $cfg.version)','$setup=Join-Path $env:GITHUB_WORKSPACE "dist/Akula_0.3.20_x64-setup.exe"'],
    ['lock',`name = "aquilum-core"\nversion = "${current}"`,`name = "aquilum-core"\nversion = "${future}"`],
    ['lock',`name = "aquilum-native"\nversion = "${current}"`,`name = "aquilum-native"\nversion = "${future}"`],
    ['build','CARGO_PKG_VERSION','FAKE_VERSION'],
    ['verifier','signed!=embedded || embedded<=installed','signed==embedded && embedded>installed'],
    ['workflow','VIDEO18C02B05_GENUINE_PE_RAISE_PASS','FAKE_SUCCESS'],
    ['workflow','$_ -lt [version]$sourceVersion','$_ -le [version]$sourceVersion'],
    ['workflow','$global:LASTEXITCODE=0','$global:LASTEXITCODE=1'],
    ['workflow','cargo build --release --locked -p aquilum-native','echo fake build'],
    ['workflow',"cancel-in-progress: ${{ github.event_name == 'pull_request' }}","cancel-in-progress: true"]
  ]){
    const value=source[k].replaceAll(from,to);
    assert.notEqual(value,source[k],'VIDEO18C02B05_HOLD: source mutation absent '+k);
    assert.throws(()=>gate({...source,[k]:value}),/VIDEO18C02B05_HOLD/);
  }
});
