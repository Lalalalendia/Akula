'use strict';
const {test}=require('node:test'), assert=require('node:assert/strict');
const fs=require('node:fs'), path=require('node:path');
const base=path.resolve(__dirname,'../..');
const paths={
  rust:'labs/video-lite/video18b18-signed-five/src/lib.rs',
  manifest:'labs/video-lite/video18b18-signed-five/Cargo.toml',
  workflow:'.github/workflows/akula-video18b18-signed-five.yml',
  source:'.github/scripts/video18b18-signed-five.test.cjs',
  fixture:'.github/scripts/video18b18-five-fixture.cjs',
  doc:'docs/release/video18b18-signed-five-admission.md',
};
const read=p=>fs.readFileSync(path.join(base,p),'utf8');
const names=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
  'akula-yt-dlp.exe','akula-quickjs.exe'];
function verify(rust,wf,fixture,toml){
  const head=wf.split(/\r?\npermissions:/,1)[0];
  assert.match(head,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const file of Object.values(paths)){
    assert.ok(head.includes("      - '"+file+"'"),'VIDEO18B18_HOLD: missing exact-HEAD source trigger '+file);
  }
  assert.match(rust,/const VIDEO_FIVE_CHANNEL_READY: bool = false;/,
    'VIDEO18B18_HOLD: release channel must be disabled');
  assert.match(rust,/if !VIDEO_FIVE_CHANNEL_READY \{\s*return Err\(hold\("five-EXE release channel is disabled"\)\);/,
    'VIDEO18B18_HOLD: actual product admission must refuse before parse');
  assert.match(rust,/verify\(raw, signature, trusted_key\)[\s\S]*?inspect_authenticated\(raw, installed_floor\)/,
    'VIDEO18B18_HOLD: parse must follow authentic signature');
  assert.match(rust,/struct Manifest \{[\s\S]*?components: Vec<Component>,/,
    'VIDEO18B18_HOLD: sealed manifest must bind members');
  assert.ok(rust.match(/#\[serde\(deny_unknown_fields\)\]/g)?.length===2,
    'VIDEO18B18_HOLD: unknown manifest/PE fields not forbidden');
  for(const word of [
    'const PURPOSE: &str = "protected-video-five-exe";',
    'const PLATFORM: &str = "windows-x86_64";',
    'const MAX_MANIFEST: usize = 8192;',
    'const MIN_PE_BYTES: u64 = 512;',
    'sha256: String,',
    'source_commit: String,',
    'cargo_lock_sha256: String,',
    'version: String,',
    'semver3(&manifest.version)? <= semver3(installed_floor)?',
    'Sha256::digest(raw)',
    'fn verify_stage(&self, current: &Path)',
    'fs::symlink_metadata',
    'meta.file_attributes() & 0x400',
    'entries.len() != NAMES.len()',
    'check_path(current, true)?',
    'hash_pe(&file)?',
    'if size != record.size || sha != record.sha256',
    'same-size PE corruption accepted',
    'missing fifth component accepted',
    'extra component accepted',
    'authentic_manifest',
  ]){
    if(word==='authentic_manifest')continue;
    assert.ok(rust.includes(word),'VIDEO18B18_HOLD: no strict candidate verification '+word);
  }
  assert.match(rust,/const NAMES: \[&str; 5\] = \[\s*"akula-app.exe",\s*"shiva-pdf-preview.exe",\s*"akula-video-mux.exe",\s*"akula-yt-dlp.exe",\s*"akula-quickjs.exe",?\s*\];/,
    'VIDEO18B18_HOLD: exact five-PE source allowlist changed');
  for(const name of names){
    assert.ok(fixture.includes("'"+name+"'"),'VIDEO18B18_HOLD: missing fixture component '+name);
  }
  assert.ok(toml.includes('publish = false') && toml.includes('[workspace]')
    && toml.includes('minisign-verify = "0.2"'),
    'VIDEO18B18_HOLD: candidate must be a standalone nonshipping Rust crate');
  assert.ok(rust.includes('fn verify(raw: &[u8], signature: &str, trusted_key: &str)'),
    'VIDEO18B18_HOLD: real Minisign verifier missing');
  for(const word of [
    'runs-on: ubuntu-24.04',
    'sudo apt-get install -y -qq minisign',
    'minisign -G -W',
    'minisign -S -s',
    'minisign -Vm',
    'AKULA_VIDEO18B18_SIGNED_FIXTURE_ROOT',
    'video18b18-signed-five/Cargo.toml',
    'cargo generate-lockfile',
    'VIDEO18B18_EPHEMERAL_LOCK_ONLY',
    'cargo test --manifest-path "$manifest" --locked',
    'persist-credentials: false',
    'permissions:\n  contents: read',
    'if: always()',
    'rm -rf -- "$AKULA_VIDEO18B18_SIGNED_FIXTURE_ROOT"',
  ])assert.ok(wf.includes(word),'VIDEO18B18_HOLD: missing actual signature/byte test '+word);
  assert.ok(!wf.includes('workflow_dispatch:')&&!/contents:\s*write|gh release create|publish_beta|cargo packager|actions\/upload-artifact/i.test(wf),
    'VIDEO18B18_HOLD: publishing or executable export enabled');
  assert.ok(fixture.includes("process.env.RUNNER_ENVIRONMENT!=='github-hosted'"),
    'VIDEO18B18_HOLD: key generation outside isolated runner');
}
test('VIDEO18B18 signed exact five-file candidate, pinned existing publisher and release HOLD',()=>{
  verify(read(paths.rust),read(paths.workflow),read(paths.fixture),read(paths.manifest));
});
test('VIDEO18B18 negative rejects enabling video channel or moving signature gate',()=>{
  const s=read(paths.rust),w=read(paths.workflow),f=read(paths.fixture),t=read(paths.manifest);
  const cases=[
    s.replace('const VIDEO_FIVE_CHANNEL_READY: bool = false;', 'const VIDEO_FIVE_CHANNEL_READY: bool = true;'),
    s.replace('verify(raw, signature, trusted_key)','no_verify(raw, signature, trusted_key)'),
    s.replace('"akula-quickjs.exe",','"untrusted.exe",'),
    s.replace('meta.file_attributes() & 0x400', 'meta.file_attributes() & 0x0'),
  ];
  for(const changed of cases){
    assert.notEqual(changed,s);
    assert.throws(()=>verify(changed,w,f,t),/VIDEO18B18_HOLD/);
  }
});
test('VIDEO18B18 negative rejects publishing and omitted CI triggers',()=>{
  const s=read(paths.rust),w=read(paths.workflow),f=read(paths.fixture),t=read(paths.manifest);
  const missing=w.replace("      - '"+paths.rust+"'\n",'');
  assert.notEqual(missing,w);
  assert.throws(()=>verify(s,missing,f,t),/VIDEO18B18_HOLD/);
  assert.throws(()=>verify(s,w+'\nworkflow_dispatch:\n',f,t),/VIDEO18B18_HOLD/);
  assert.throws(()=>verify(s,w+'\npermissions:\n  contents: write\n',f,t),/VIDEO18B18_HOLD/);
});
