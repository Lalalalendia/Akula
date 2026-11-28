'use strict';
const assert=require('node:assert/strict');
const installedRoute = require('./video-tools-installed-contract.cjs');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const floor=require(path.join(root,'scripts/video-tools-01o-signed-floor.cjs'));
const paths={
  rust:'labs/video-lite/src/bin/dash.rs',
  identity:'labs/video-lite/src/version_01o.rs',
  cargo:'labs/video-lite/Cargo.toml',
  runner:'scripts/windows-video-tools-01o-real-mux-floor.ps1',
  verify:'scripts/video-tools-01o-signed-floor.cjs',
  flow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  trust:'aquilum-app/native/src/video/tools_package.rs',
  ...installedRoute.paths,
  doc:'docs/release/video-tools-01o-real-mux-floor.md'
};
const load=()=>Object.fromEntries(Object.entries(paths).map(([k,v])=>
  [k,fs.readFileSync(path.join(root,v),'utf8')]));
const hold=(v,m)=>assert.ok(v,'VIDEO_TOOLS_01O_HOLD: '+m);
function check(s){
  hold(s.cargo.includes('lab_mux_020 = []')&&s.identity.includes('const VERSION: &str = "0.2.0";')&&
    s.identity.includes('const VERSION: &str = "0.1.0";')&&
    s.rust.includes('version_01o::probe_identity()'),
    'source-controlled two distinct build identities missing');
  hold(s.trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";')&&
    s.trust.includes('false // a future, separately reviewed release gate must replace this constant'),
    'production publisher key or release HOLD bypassed');
  installedRoute.checkInstalledRoute(s, hold);
  for(const t of [
    'cp.execFileSync(signer,','sha(content)!==b.sha256',
    'if(!newer(candidate.version,current.version))return',
    "candidate.binaries[0].sha256===current.binaries[0].sha256",
    'source_commit!==current.source_commit',
    "process.env.GITHUB_EVENT_NAME!=='pull_request'",
    "VIDEO_TOOLS_01O_SIGNED_FLOOR_"
  ])hold(s.verify.includes(t),'signed independent floor preflight missing '+t);
  for(const t of [
    '--features lab_mux_020','--locked --release',
    "foreach($upstream in @{'akula-yt-dlp.exe'=$YtDlpPath;'akula-quickjs.exe'=$QuickJsPath}.GetEnumerator())",
    'Where-Object {$_.name -ceq $upstream.Key}',
    "Assert-Identity (Join-Path $three['old'] 'akula-video-mux.exe') '0.1.0'",
    "Assert-Identity (Join-Path $three['new'] 'akula-video-mux.exe') '0.2.0'",
    'if($oldSha -ceq $newSha)',
    "Assert-Native 'valid' '0.1.0'",
    "Assert-Native 'status-incompatible' '0.2.0'",
    "$five['new'] $root $pub $signer 'ALLOW'",
    "$five['old'] $root $pub $signer 'DENY'",
    'if($direct.ExitCode -ne 77)',
    'bytes_unchanged=6 release=0',
    'VIDEO_TOOLS_01O_REAL_MUX_FLOOR_PASS'
  ])hold(s.runner.includes(t),'real Windows two-PE acceptance omitted '+t);
  for(const t of [
    "      - 'scripts/windows-video-tools-01o-real-mux-floor.ps1'",
    "      - '.github/scripts/video-tools-01o-real-mux.test.cjs'",
    "      - 'docs/release/video-tools-01o-real-mux-floor.md'",
    'node --test .github/scripts/video-tools-01o-real-mux.test.cjs',
    'VIDEO_TOOLS_01O_REAL_MUX_FLOOR_PASS',
    'runs-on: windows-2025','persist-credentials: false',
    'permissions:\n  contents: read'
  ])hold(s.flow.includes(t),'exact-HEAD non-release CI missing '+t);
  hold(!/workflow_dispatch:|^  push:|contents: write|gh release create/m.test(s.flow),
    'nonshipping workflow may publish');
  for(const t of ['ephemeral','RELEASE HOLD','distinct','manual','after uninstall','Windows 10/11'])
    hold(s.doc.includes(t),'release limitation omitted '+t);
}
test('numeric version ordering is canonical and not string-lexical',()=>{
  assert.equal(floor.newer('0.10.0','0.2.0'),true);
  assert.equal(floor.newer('0.2.0','0.10.0'),false);
  assert.equal(floor.newer('0.2.0','0.1.0'),true);
  assert.equal(floor.newer('0.1.0','0.2.0'),false);
  assert.equal(floor.newer('0.2.0','0.2.0'),false);
  for(const bad of ['0.01.0','0.2','0.2.0-preview','4294967296.0.0','-1.0.0','1.2.03']){
    assert.throws(()=>floor.version(bad),/VIDEO_TOOLS_01O_HOLD/);
  }
});
test('signed manifest cannot fake a different executable as a new version',()=>{
  const base={
    schema:1,product:'Akula Video Tools',version:'0.1.0',platform:'windows-x86_64',
    cargo_lock_sha256:'a'.repeat(64),source_commit:'a'.repeat(40),
    binaries:['akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'].map((name,i)=>({
      name,sha256:(i+1).toString().repeat(64),size:1024
    }))
  };
  const current=floor.identity(base),fake=floor.identity({...base,version:'0.2.0'});
  assert.throws(()=>floor.checkFloor(fake,current),/VIDEO_TOOLS_01O_HOLD/);
  const actual=floor.identity({...fake,binaries:[{...fake.binaries[0],sha256:'f'.repeat(64)},...fake.binaries.slice(1)]});
  assert.equal(floor.checkFloor(actual,current),'ALLOW');
  assert.equal(floor.checkFloor(current,actual),'DENY');
  assert.throws(()=>floor.identity({...fake,malicious:true}),/VIDEO_TOOLS_01O_HOLD/);
});
test('non-shipping real Windows acceptance and release HOLD are mandatory',()=>check(load()));
test('source mutations break the contract',()=>{
  const s=load();
  for(const [k,from,to] of [
    ['identity','const VERSION: &str = "0.2.0";','const VERSION: &str = "0.1.0";'],
    ['runner','Where-Object {$_.name -ceq $upstream.Key}','Where-Object {$_.name -ceq $input.Key}'],
    ['runner','if($oldSha -ceq $newSha)','if($false)'],
    ['runner',"$five['old'] $root $pub $signer 'DENY'","$five['old'] $root $pub $signer 'ALLOW'"],
    ['runner','if($direct.ExitCode -ne 77)','if($false)'],
    ['verify','if(!newer(candidate.version,current.version))return','if(false)return'],
    ['verify','sha(content)!==b.sha256','false'],
    ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "lab";'],
    ['flow','permissions:\n  contents: read','permissions:\n  contents: write']
  ]) {
    hold(s[k].includes(from),'mutation anchor missing '+k+': '+from);
    assert.throws(()=>check({...s,[k]:s[k].replace(from,to)}),/VIDEO_TOOLS_01O_HOLD/);
  }
});
