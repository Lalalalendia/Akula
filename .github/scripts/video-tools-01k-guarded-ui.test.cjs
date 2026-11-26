'use strict';
// VIDEO-TOOLS-01k: negative source and locale contracts for installed Windows UI.
const assert=require('node:assert/strict');
const installedRoute = require('./video-tools-installed-contract.cjs');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const files={
  ui:'aquilum-app/native/src/app/editor_requests/video_stock_admission.rs',
  click:'aquilum-app/native/src/app/editor_requests/video.rs',
  probe:'aquilum-app/native/src/video/tools_package_probe.rs',
  version:'aquilum-app/native/src/video/tools_package_probe_version.rs',
  mod:'aquilum-app/native/src/video/mod.rs',
  ...installedRoute.paths,
  queueTests:'aquilum-app/native/src/video/queue_01k_tests.rs',
  trust:'aquilum-app/native/src/video/tools_package.rs',
  stock:'aquilum-app/native/packaging/akula-windows.json',
  ru:'aquilum-app/native/assets/locales/ru.json',
  en:'aquilum-app/native/assets/locales/en.json',
  workflow:'.github/workflows/akula-video-tools-01a-isolated-lab.yml',
  document:'docs/release/video-tools-01k-user-status.md'
};
const load=()=>Object.fromEntries(Object.entries(files).map(([k,p])=>[k,
  fs.readFileSync(path.join(root,p),'utf8')]));
const ensure=(yes,msg)=>assert.ok(yes,'VIDEO_TOOLS_01K_HOLD: '+msg);
function check(x){
  const {ui,click,probe,version,mod,queue,queueTests,trust,stock,ru,en,workflow,document}=x;
  assert.deepEqual(JSON.parse(stock).binaries.map(b=>b.path),['akula-app','shiva-pdf-preview'],
    'VIDEO_TOOLS_01K_HOLD: stock installer must remain only two EXEs');
  ensure(trust.includes('pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";')&&
    trust.includes('false // a future, separately reviewed release gate must replace this constant'),
    'publisher root and shipping HOLD must remain disabled');
  ensure(mod.includes('pub(crate) mod tools_package_probe;')&&
    mod.includes('all(windows, any(test, feature = "installed"))'),
    'native-only OS discovery must be exposed for read-only status');
  for(const token of [
    'pub(crate) enum ToolsDiscovery','pub(crate) fn probe_installed_video_tools()',
    'Missing','Unsafe','PublisherNotApproved','IncompatibleVersion','ReadOnlyAttested',
    'windows_discovery::installed_root()','windows_discovery::check_installed_acl',
    'APPROVED_VIDEO_TOOLS_KEY.is_empty() || !shipping_admission_enabled()',
    'verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY)',
    'classify_authenticated_version(&attested.version)'
  ])ensure(probe.includes(token),'native signed status incomplete: '+token);
  ensure(version.includes('"0.1.0" => ToolsDiscovery::ReadOnlyAttested') &&
    version.includes('_ => ToolsDiscovery::IncompatibleVersion'),
    'authenticated exact signed-version classifier missing');
  const localeKeys=[];
  for(const [v,base] of [
    ['Missing','Missing'],['Invalid','Unsafe'],['Downloading(_)','Downloading'],
    ['Failed(_)','DownloadFailed'],['Installed','Attested']
  ]){
    ensure(ui.includes('Status::'+v+' => ('),'native status '+v+' omitted from UI');
    localeKeys.push('videoTools'+base+'Title','videoTools'+base);
  }
  for(const key of localeKeys){
    for(const [language,content] of [['ru',ru],['en',en]]){
      const value=JSON.parse(content).editor[key];
      ensure(typeof value==='string'&&value.trim().length>15,
        language+' locale missing status copy for '+key);
    }
  }
  for(const token of [
    'if self.optional_video_tools_notice(root) {',
    'mark("NOTICE_BLOCKED"); return; }',
    'crate::video::queue::enqueue('
  ])ensure(click.includes(token),'click-to-notice-to-queue order incomplete');
  ensure(click.indexOf('optional_video_tools_notice')<click.indexOf('queue::enqueue'),
    'click may enqueue before protected status notice');
  for(const token of [
    'optional_ytdlp::status(&self.workspace.data_dir)', 'notice_keys(',
    'if matches!(state, Status::Installed) {\n                return false;',
    't(title)','t(body)','ConfirmDialog::new(',
    'self.popup = Some(Popup::PdfNotice { dialog });',
    'return true;', 'not(feature = "video-ui-e2e")'
  ])ensure(ui.includes(token),'installed UI native status flow missing '+token);
  ensure(!ui.includes('external_ytdlp_visible')&&!ui.includes('AKULA_YT_DLP_BIN')&&
    !ui.includes('std::env::var_os("PATH")'),
    'PATH/env hints cannot authorize normal installed UI');
  installedRoute.checkInstalledRoute(x, ensure);
  ensure(queue.includes('mod installed_tests;')&&queue.includes('#[path = "queue_01k_tests.rs"]'),
    'installed queue must register its standalone native test');
  for(const token of [
    'installed_download_refusal_creates_no_job_or_local_bytes',
    'assert!(matches!(status(url), Some(Status::Failed(_))))',
    'std::fs::read_dir(temp.path()).unwrap().count(), 0'
  ])ensure(queueTests.includes(token),'Windows no-job/no-local-bytes test missing '+token);
  for(const token of [
    "      - 'aquilum-app/native/src/video/tools_package_probe.rs'",
    "      - '.github/workflows/akula-video-tools-01a-isolated-lab.yml'",
    'node --test .github/scripts/video-tools-01k-guarded-ui.test.cjs',
    'cargo check --locked --manifest-path aquilum-app/Cargo.toml -p aquilum-native --features installed',
    'video::queue::installed_tests::installed_download_refusal_creates_no_job_or_local_bytes',
    'optional_download_states_have_real_localized_messages',
    'VIDEO_TOOLS_01K_INSTALLED_WINDOWS_PASS',
    'runs-on: windows-2025','permissions:\n  contents: read','persist-credentials: false'
  ])ensure(workflow.includes(token),'required Windows CI proof missing '+token);
  ensure(!/workflow_dispatch:|^\s+push:|contents:\s*write|gh release create|publish_beta/m.test(workflow),
    '01k CI cannot publish or deploy');
  ensure(document.includes('RELEASE HOLD')&&document.includes('ReadOnlyAttested')&&
    document.includes('no real Video Tools install'),'explicit release limitations missing');
}
test('installed Windows Video Tools status is diagnostic, never executable authority',()=>check(load()));
test('mutations of UI/native guard, locales, or Windows acceptance cannot silently pass',()=>{
  const s=load();
  const cases=[
    ['ui','optional_ytdlp::status(&self.workspace.data_dir)','TrustPATH()'],
    ['ui','Status::Invalid => (','Status::Invalid /* fake */ => ('],
    ['ui','return true;','return false;'],
    ['click','if self.optional_video_tools_notice(root) {','if false {'],
    ['queue','optional_ytdlp::verified_executable','optional_ytdlp::unchecked_executable'],
    ['queueTests','assert!(matches!(status(url), Some(Status::Failed(_))))','assert!(true)'],
    ['dispatch','quickjs.ok_or(', 'quickjs.unwrap_or('],
    ['leaf','VIDEO_TOOLS_01K_HOLD: publisher-approved process launch is unavailable','allow execution'],
    ['version','"0.1.0" => ToolsDiscovery::ReadOnlyAttested','"0.1.0" => ToolsDiscovery::IncompatibleVersion'],
    ['trust','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";','pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "ci";'],
    ['ru','"videoToolsMissingTitle"', '"missingRussianTitle"'],
    ['en','"videoToolsUnsafeTitle"', '"missingEnglishTitle"'],
    ['workflow','cargo check --locked --manifest-path aquilum-app/Cargo.toml -p aquilum-native --features installed','echo success'],
    ['workflow','node --test .github/scripts/video-tools-01k-guarded-ui.test.cjs','echo skip'],
    ['workflow','permissions:\n  contents: read','permissions:\n  contents: write']
  ];
  for(const [key,from,to] of cases){
    ensure(s[key].includes(from),'missing mutation anchor '+key+' '+from);
    assert.throws(()=>check({...s,[key]:s[key].replace(from,to)}),/VIDEO_TOOLS_01K_HOLD/);
  }
});
