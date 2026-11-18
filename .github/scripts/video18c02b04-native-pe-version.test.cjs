'use strict';
const { test }=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const paths={
  build:'aquilum-app/native/build.rs',
  rust:'labs/video-lite/video18c02b04-native-pe-version.rs',
  workflow:'.github/workflows/akula-video18c02b04-native-pe-version.yml',
  doc:'docs/release/video18c02b04-native-pe-version.md',
};
const read=k=>fs.readFileSync(path.join(root,paths[k]),'utf8');
function gate(build,rust,workflow,doc){
  for(const token of [
    'std::env::var("CARGO_PKG_VERSION")',
    'entry(&mut res, 16, 1, 0x1030, &win32_fixed_version_info(&package_version))',
    'fn win32_fixed_version_info(version: &str)',
    '"VS_VERSION_INFO\\0".encode_utf16()',
    '0xFEEF04BDu32'
  ])assert.ok(build.includes(token),'VIDEO18C02B04_HOLD: missing true native version resource '+token);
  for(const token of [
    'GetFileVersionInfoSizeW','GetFileVersionInfoW','VerQueryValueW',
    '0xFEEF04BD','fixed[2]!=fixed[4]','embedded!=source',
    'signed!=embedded || embedded<=installed',
    'EXPECTED_HEAD_SHA','RUNNER_ENVIRONMENT','GITHUB_EVENT_NAME',
    'only exact source-built native EXE admitted',
    'VIDEO18C02B04_NATIVE_PE_IDENTITY_PASS','VIDEO18C02B04_STRICT_VERSION_FLOOR_PASS'
  ])assert.ok(rust.includes(token),'VIDEO18C02B04_HOLD: missing native version admission '+token);
  for(const token of [
    "node --test .github/scripts/video18c02b04-native-pe-version.test.cjs",
    'cargo metadata --locked --no-deps --format-version 1',
    'cargo build --release --locked -p aquilum-native',
    'VIDEO18C02B04_FALSE_VERSION_DENIED_PASS',
    'VIDEO18C02B04_NATIVE_PE_IDENTITY_PASS',
    '$deniedExit=$LASTEXITCODE',
    'VIDEO18C02B04_HOLD: signed label is not a genuine newer native PE version',
    '$global:LASTEXITCODE=0',
    'rustc --edition=2024 --test',
    'git rev-parse HEAD','EXPECTED_HEAD_SHA',
    'github.event.pull_request.head.sha',
    paths.rust,paths.build
  ])assert.ok(workflow.includes(token),'VIDEO18C02B04_HOLD: missing CI verification '+token);
  assert.ok(!/workflow_dispatch:|contents:\s*write|gh release create/.test(workflow),
    'VIDEO18C02B04_HOLD: must never be a release workflow');
  for(const token of ['RELEASE HOLD','0.3.20','0.3.21','not a version upgrade']){
    assert.ok(doc.includes(token),'VIDEO18C02B04_HOLD: missing version limitation '+token);
  }
}
test('Windows native PE version is compiled from actual Cargo package version',()=>{
  gate(read('build'),read('rust'),read('workflow'),read('doc'));
});
test('Fail closed when version, Windows identity, HEAD or false-label refusal is bypassed',()=>{
  const b=read('build'),r=read('rust'),w=read('workflow'),d=read('doc');
  for(const [file,from,to] of [
    ['build','"CARGO_PKG_VERSION"','"FORGED_VERSION"'],
    ['build','entry(&mut res, 16, 1, 0x1030, &win32_fixed_version_info(&package_version))','// no RT_VERSION'],
    ['rust','signed!=embedded || embedded<=installed','signed==embedded && embedded>installed'],
    ['rust','embedded!=source','embedded==source'],
    ['rust','GetFileVersionInfoW','FakeVersionQuery'],
    ['rust','RUNNER_ENVIRONMENT','NO_RUNNER_GUARD'],
    ['rust','only exact source-built native EXE admitted','allow any path'],
    ['workflow','$global:LASTEXITCODE=0','$global:LASTEXITCODE=1'],
    ['workflow','VIDEO18C02B04_HOLD: signed label is not a genuine newer native PE version','FAKE_REJECTION'],
    ['workflow','VIDEO18C02B04_FALSE_VERSION_DENIED_PASS','PHANTOM_VERSION_ALLOWED'],
    ['workflow','cargo build --release --locked -p aquilum-native','echo fake-built'],
  ]){
    const before=file==='build'?b:file==='rust'?r:w;
    const changed=before.replaceAll(from,to);
    assert.notEqual(changed,before,'VIDEO18C02B04_HOLD: negative mutation absent');
    assert.throws(()=>gate(file==='build'?changed:b,file==='rust'?changed:r,
      file==='workflow'?changed:w,d),/VIDEO18C02B04_HOLD/,
      'mutation must fail closed: '+file+' '+from);
  }
});
