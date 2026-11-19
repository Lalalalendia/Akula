'use strict';
// VIDEO-TOOLS-01l: source mutation contracts; real UI accepted separately on Win32.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const paths={
 cargo:'aquilum-app/native/Cargo.toml',
 video:'aquilum-app/native/src/app/editor_requests/video.rs',
 mouse:'aquilum-app/native/src/ui/editor/widget/media/mouse.rs',
 events:'aquilum-app/native/src/ui/editor/widget/widget_events.rs',
 preview:'aquilum-app/native/src/app/embeds/youtube.rs',
 module:'aquilum-app/native/src/video/mod.rs',
 witness:'aquilum-app/native/src/video/ui_acceptance.rs',
 queue:'aquilum-app/native/src/video/queue.rs',
 leaf:'aquilum-app/native/src/video/light.rs',
 trust:'aquilum-app/native/src/video/tools_package.rs',
 stock:'aquilum-app/native/packaging/akula-windows.json',
 ps:'scripts/windows-video-tools-01l-guarded-gui.ps1',
 sentinel:'scripts/video-tools-01l-child-sentinel.rs',
 flow:'.github/workflows/akula-video-tools-01l-guarded-gui.yml',
 doc:'docs/release/video-tools-01l-guarded-gui.md'
};
const read=()=>Object.fromEntries(Object.entries(paths).map(([k,p])=>[k,fs.readFileSync(path.join(root,p),'utf8')]));
const hold=(yes,message)=>assert.ok(yes,'VIDEO_TOOLS_01L_HOLD: '+message);
function check(x){
 hold(x.cargo.includes('video-tools-gui-e2e = []'),'observation feature must be empty');
 hold(x.cargo.includes('video-ui-e2e = ["video-rust-bundle"]'),'prior download feature changed');
 const installed=x.cargo.match(/^installed = \[([^\n]+)\]/m);
 hold(installed&&!/video-tools-gui-e2e|video-ui-e2e|video-rust-bundle/.test(installed[1]),
    'nonshipping feature enabled in ordinary installer');
 const cfg='any(feature = "video-ui-e2e", feature = "video-tools-gui-e2e")';
 for(const file of ['module','video','mouse','events','preview'])
    hold(x[file].includes(cfg),'actual OS pointer observation missing '+file);
 for(const token of ['NOTICE_BLOCKED','HOVER_DOWNLOAD','PRESS_DOWNLOAD','WINDOW_POINTER_DOWN'])
    hold(x.witness.includes(token),'native witness not permitted '+token);
 hold(x.video.includes('mark("NOTICE_BLOCKED")')&&
      x.video.indexOf('optional_video_tools_notice')<x.video.indexOf('queue::enqueue('),
      'download must abort before queue');
 hold(x.queue.includes('if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))')&&
      x.leaf.includes('VIDEO_TOOLS_01K_HOLD: publisher-approved process launch is unavailable'),
      'installed queue or executable launch gate weakened');
 hold(x.trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";')&&
      x.trust.includes('false // a future, separately reviewed release gate must replace this constant'),
      'publisher key or shipping approval silently enabled');
 hold(JSON.parse(x.stock).binaries.map(b=>b.path).join(',')==='akula-app,shiva-pdf-preview',
      'public two-EXE installer modified');
 hold(x.preview.includes('blank_preview()'),'GUI test can request YouTube thumbnail CDN');
 for(const token of [
    'RUNNER_ENVIRONMENT','EXPECTED_HEAD_SHA','Administrator',
    'Akula Video Tools','ProgramFiles=$spoof','Icacls @($root',
    'S-1-5-32-544','S-1-5-32-545:M','Remove-Item -LiteralPath $root',
    'video16-events.log','VideoTools01lPointer','SendInput(1,down',
    'GetWindowThreadProcessId','Editor-Bounds','UIAutomationClient',
    'FindAll([Windows.Automation.TreeScope]::Descendants',
    'CLICK_DOWNLOAD','NOTICE_BLOCKED','HOVER_DOWNLOAD',
    'PRESS_DOWNLOAD','WINDOW_POINTER_DOWN','Assert-Dialog',
    'Assert-No-Launch','AKULA_VIDEO_TOOLS_01L_CHILD_MARKER',
    "Scenario 'missing' 'Akula Video Tools are not installed'",
    "Scenario 'publisher-not-approved' 'Video Tools publisher is not yet approved'",
    "Scenario 'unsafe' 'Video Tools installation did not pass verification'",
    'VIDEO_TOOLS_01L_REAL_GUI_PASS real_pointer=3'
 ])hold(x.ps.includes(token),'genuine Windows GUI test proof missing '+token);
 hold(!/Invoke-WebRequest|Invoke-RestMethod|Start-BitsTransfer|gh release create/.test(x.ps),
     'GUI test cannot fetch or publish external binaries');
 for(const token of ['GITHUB_ACTIONS','AKULA_VIDEO_TOOLS_01L_CHILD_MARKER',
     'write_all','video-tools-01l-child-executed.flag'])
     hold(x.sentinel.includes(token),'launch-detection child missing '+token);
 for(const token of [
    'runs-on: windows-2025','persist-credentials: false',
    'permissions:\n  contents: read','video-tools-gui-e2e',
    'node --test .github/scripts/video-tools-01l-guarded-gui.test.cjs',
    'cargo build --release --locked',
    "rustc --edition=2024 -O (Join-Path $env:GITHUB_WORKSPACE 'scripts/video-tools-01l-child-sentinel.rs')",
    'powershell.exe -NoProfile -STA',
    'VIDEO_TOOLS_01L_WINDOWS_ACCEPTED_NONRELEASE','EXPECTED_HEAD_SHA:'
 ])hold(x.flow.includes(token),'dedicated Windows acceptance missing '+token);
 const prFiles=x.flow.split('\n').filter(l=>l.startsWith("      - '")&&l.endsWith("'"))
    .map(l=>l.trim().slice(3,-1));
 hold(JSON.stringify(prFiles)===JSON.stringify([
    'scripts/windows-video-tools-01l-guarded-gui.ps1',
    'scripts/video-tools-01l-child-sentinel.rs',
    '.github/scripts/video-tools-01l-guarded-gui.test.cjs',
    'docs/release/video-tools-01l-guarded-gui.md',
    '.github/workflows/akula-video-tools-01l-guarded-gui.yml'
 ]),'new CI cannot increase fanout on existing source');
 hold(!/workflow_dispatch:|^  push:|contents: write|gh release create|publish_beta/m.test(x.flow),
     'nonshipping workflow can publish or deploy');
 hold(x.doc.includes('ReadOnlyAttested')&&x.doc.includes('PublisherNotApproved')&&
      x.doc.includes('RELEASE HOLD'),'scope omits unavailable states and release hold');
}
test('real installed GUI observes denial without enabling runtime execution',()=>check(read()));
test('negative mutations of click, protected root and CI proof are blocked',()=>{
 const s=read();
 for(const [file,from,to] of [
   ['cargo','video-tools-gui-e2e = []','video-tools-gui-e2e = ["video-ui-e2e"]'],
   ['video','mark("NOTICE_BLOCKED")','mark("NOTICE_OK")'],
   ['mouse','feature = "video-tools-gui-e2e"','feature = "video-tools-fake"'],
   ['witness','"NOTICE_BLOCKED"','"NOTICE_ALLOWED"'],
   ['queue','if cfg!(all(windows, feature = "installed", not(feature = "video-ui-e2e")))','if false'],
   ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "ci";'],
   ['ps','SendInput(1,down','SendInput(0,down'],
   ['ps','Icacls @($root','Write-Output fake-acl @($root'],
   ['ps','Assert-No-Launch','Skip-No-Launch'],
   ['ps','Assert-Dialog','Skip-Dialog'],
   ['ps','RUNNER_ENVIRONMENT','FAKE_ENVIRONMENT'],
   ['sentinel','write_all','do_nothing'],
   ['flow','powershell.exe -NoProfile -STA','echo skipped'],
   ['flow','permissions:\n  contents: read','permissions:\n  contents: write']
 ]){
    hold(s[file].includes(from),'mutation anchor missing '+file+' '+from);
    assert.throws(()=>check({...s,[file]:s[file].replaceAll(from,to)}),/VIDEO_TOOLS_01L_HOLD/, 'mutation must fail: '+file+' '+from);
 }
});
