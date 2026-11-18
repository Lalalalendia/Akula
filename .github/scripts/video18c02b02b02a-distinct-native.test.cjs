'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'../..');
const p={
  script:'scripts/windows-video18c02b02b02a-distinct-native.ps1',
  workflow:'.github/workflows/akula-video18b16-versioned-bridge.yml',
  doc:'docs/release/video18c02b02b02a-distinct-source-built-pe.md',
  contract:'.github/scripts/video18c02b02b02a-distinct-native.test.cjs',
};
const read=n=>fs.readFileSync(path.join(root,p[n]),'utf8');
function check(script,workflow,doc){
  const paths=workflow.split(/\r?\npermissions:/,1)[0];
  for(const filename of Object.values(p)){
    assert.ok(paths.includes("      - '"+filename+"'"),
      'VIDEO18C02B02B02A_HOLD: missing trigger '+filename);
  }
  for(const marker of [
    "ec6d6d48a78e60fca7a6f810b825418500bad64b",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    '(git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA',
    '(git -C $oldTree rev-parse HEAD).Trim() -cne $oldCommit',
    'Assert-RealFile $oldApp', 'Assert-RealFile $newApp',
    '[IO.FileAttributes]::ReparsePoint',
    "aquilum-app/native/src/updater/windows_first_gui_ack.rs",
    "HEAD:aquilum-app/native/Cargo.toml", 'HEAD:aquilum-app/Cargo.lock',
    "'0\\.3\\.20'", '$oldHash -ceq $newHash',
    "VIDEO18C02B02B02A_DISTINCT_SOURCE_PE_PASS",
    '[System.Diagnostics.ProcessStartInfo]::new()',
    '$start.UseShellExecute=$false', '$process.StartTime.ToUniversalTime()',
    '$process.MainWindowHandle -ne [IntPtr]::Zero',
    "$process.MainWindowTitle.Contains('Akula')",
    '$pinned.StartTime.ToUniversalTime() -ne $started',
    'if(!$process.CloseMainWindow() -or !$process.WaitForExit(20000))',
    '(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash',
    "VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS",
    '$process.Kill($true)',
  ]) assert.ok(script.includes(marker),'VIDEO18C02B02B02A_HOLD: missing fail-closed check '+marker);
  assert.ok(script.includes("!$oldApp.StartsWith(($temp.TrimEnd('\\')+'\\')"),
    'VIDEO18C02B02B02A_HOLD: old executable escaped runner temp');
  assert.ok(script.includes("!$newApp.StartsWith(($repo.TrimEnd('\\')+'\\')"),
    'VIDEO18C02B02B02A_HOLD: new executable escaped checkout');
  for(const marker of [
    "repository: Lalalalendia/Akula",
    "ref: ec6d6d48a78e60fca7a6f810b825418500bad64b",
    'path: _akula-historical',
    'persist-credentials: false',
    'VIDEO18C02B02B02A_SOURCE_COMMIT_PIN_PASS',
    'cargo build --release --locked -p aquilum-native --features installed,video-rust-bundle,video-ui-e2e',
    'VIDEO18C02B02B02A_OLD_APP',
    'scripts/windows-video18c02b02b02a-distinct-native.ps1',
    'VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS',
    '.github/scripts/video18c02b02b02a-distinct-native.test.cjs',
    'needs: [contract, preflight]'
  ]) assert.ok(workflow.includes(marker),'VIDEO18C02B02B02A_HOLD: missing CI requirement '+marker);
  assert.match(workflow,
    /run: node --test [^\r\n]*\.github\/scripts\/video18c02b02b02a-distinct-native\.test\.cjs/,
    'VIDEO18C02B02B02A_HOLD: Node negative contract not executed');
  assert.ok(!/workflow_dispatch:|contents:\s*write|gh release create/.test(workflow),
    'VIDEO18C02B02B02A_HOLD: no release/privileged workflow');
  assert.ok(!/confirm_candidate\s*\(|publish_beta|Remove-Item.+-Recurse/.test(script),
    'VIDEO18C02B02B02A_HOLD: no release confirmation or recursive deletion');
  assert.ok(doc.includes('0.3.20')&&doc.includes('не доказывает')&&
    doc.includes('RELEASE HOLD'), 'VIDEO18C02B02B02A_HOLD: missing scope limit');
}
test('Real old/new pinned PE binaries differ and historical Akula window is visible',()=>{
  check(read('script'),read('workflow'),read('doc'));
});
test('Negative: weakened old-source pin, PE comparison, OS witness or source custody fails',()=>{
  const script=read('script'),w=read('workflow'),d=read('doc');
  for(const [field,from,to] of [
    ['script',"$oldHash -ceq $newHash",'$false'],
    ['script',"(git -C $oldTree rev-parse HEAD).Trim() -cne $oldCommit",'$false'],
    ['script',"$process.MainWindowTitle.Contains('Akula')",'$true'],
    ['script','$pinned.StartTime.ToUniversalTime() -ne $started','$false'],
    ['script','[IO.FileAttributes]::ReparsePoint','[IO.FileAttributes]::None'],
    ['workflow','ref: ec6d6d48a78e60fca7a6f810b825418500bad64b','ref: main'],
    ['workflow','path: _akula-historical','path: fake'],
    ['workflow','VIDEO18C02B02B02A_SOURCE_COMMIT_PIN_PASS','VIDEO18C02B02B02A_PRETEND_PASS'],
    ['workflow','needs: [contract, preflight]','needs: contract'],
  ]){
    const old=field==='script'?script:w;
    const modified=old.replace(from,to);
    assert.notEqual(modified,old,'VIDEO18C02B02B02A_HOLD: mutation did not occur');
    assert.throws(()=>check(field==='script'?modified:script,field==='workflow'?modified:w,d),
      /VIDEO18C02B02B02A_HOLD/);
  }
});
