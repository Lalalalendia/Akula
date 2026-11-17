'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const paths={
  rust:'labs/video-lite/video18b15_rar2_engine_bridge.rs',
  workflow:'.github/workflows/akula-video18b15-rar2-bridge.yml',
  contract:'.github/scripts/video18b15-rar2-bridge.test.cjs',
  doc:'docs/release/video18b15-rar2-engine-bridge.md'
};
const pin='32a803e67244836d2794f9bbbdf9b24deab63d7e';
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
function verify(wf,rust){
  const hdr=wf.split(/\r?\npermissions:/,1)[0];
  assert.match(hdr,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for(const file of Object.values(paths)){
    assert.ok(hdr.includes("      - '"+file+"'"),'VIDEO18B15_HOLD: missing source trigger '+file);
  }
  for(const term of ['repository: Lalalalendia/rar2',
    'ref: '+pin, "$pin='"+pin+"'",
    'git -C _rar2-pinned rev-parse HEAD',
    'cargo test --locked -p chaptera-update-engine',
    '--test video18b15_akula_five',
    'persist-credentials: false',
    'permissions:\n  contents: read']){
    assert.ok(wf.includes(term),'VIDEO18B15_HOLD: missing source custody '+term);
  }
  assert.match(rust,/const COMPONENTS: \[&str; 5\] = \[\s*"akula-app.exe",\s*"shiva-pdf-preview.exe",\s*"akula-video-mux.exe",\s*"akula-yt-dlp.exe",\s*"akula-quickjs.exe",?\s*\];/, 'VIDEO18B15_HOLD: exact five-member tuple changed');
  for(const term of ['UpdateEngine','RecoveryOutcome',
    'COMPONENTS: [&str; 5]', '"akula-app.exe"',
    '"shiva-pdf-preview.exe"','"akula-video-mux.exe"',
    '"akula-yt-dlp.exe"','"akula-quickjs.exe"',
    'exact_five','fs::symlink_metadata',
    'engine.recover()','engine.retain_previous()',
    'engine.activate_candidate()','engine.confirm_candidate()',
    'akula_unconfirmed_new_generation_rolls_back_without_partial_mix',
    'flat_nsis_layout_is_not_implicitly_compatible_with_upstream_current_tree',
    'VIDEO18B15_FLAT_NSIS_MIGRATION_HOLD']){
    assert.ok(rust.includes(term),'VIDEO18B15_HOLD: missing Akula test adapter '+term);
  }
  assert.ok(!wf.includes('workflow_dispatch:'),'VIDEO18B15_HOLD: manual workflow not approved');
  assert.ok(!/contents:\s*write|gh release create|publish_beta|cargo packager|^\s*push:/im.test(wf),
    'VIDEO18B15_HOLD: release or write authority forbidden');
  assert.ok(!/std::process::Command|ShellExecute|ProgramFiles|[\\/](?:Program Files|Windows)[\\/]/i.test(rust),
    'VIDEO18B15_HOLD: test adapter cannot execute programs or touch protected install');
}
test('VIDEO18B15 strict pinned rar2 engine and no-release contract',()=>{
  verify(read(paths.workflow),read(paths.rust));
});
test('VIDEO18B15 negative pin and five-exe identity mutations',()=>{
  const wf=read(paths.workflow),rust=read(paths.rust);
  const unpinned=wf.replace('ref: '+pin,'ref: main');
  assert.notEqual(wf,unpinned);
  assert.throws(()=>verify(unpinned,rust),/VIDEO18B15_HOLD/);
  const wrong=rust.replace('"akula-quickjs.exe"','"other.exe"');
  assert.notEqual(wrong,rust);
  assert.throws(()=>verify(wf,wrong),/VIDEO18B15_HOLD/);
});
test('VIDEO18B15 negative release and missing trigger mutations',()=>{
  const wf=read(paths.workflow),rust=read(paths.rust);
  const noTrigger=wf.replace("      - '"+paths.rust+"'\n",'');
  assert.notEqual(wf,noTrigger);
  assert.throws(()=>verify(noTrigger,rust),/VIDEO18B15_HOLD/);
  assert.throws(()=>verify(wf+'\nworkflow_dispatch:\n',rust),/VIDEO18B15_HOLD/);
});
