'use strict';
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {test}=require('node:test');
const root=path.resolve(__dirname,'../..');
const {validateMetadata,validateText,gitBlobSha}=require('./video-tools-01q-executable-license-evidence.cjs');
const allow=()=>JSON.parse(fs.readFileSync(path.join(root,'docs/release/video18b04-five-exe-allowlist.json'),'utf8'));
const wf=()=>fs.readFileSync(path.join(root,'.github/workflows/akula-video18b09-windows-license-triage.yml'),'utf8');
test('source and combined Windows executable licenses are intentionally different',()=>{
  const {yt,q,mux}=validateMetadata(allow());
  assert.equal(yt.source_license,'Unlicense');
  assert.equal(yt.license,'GPL-3.0-or-later');
  assert.equal(q.license,'MIT');
  assert.equal(mux.license,'AGPL-3.0-only');
  assert.equal(yt.tag_commit_sha,'f264a6cbd1f630894da0f368df00f8aae50140e0');
  assert.equal(q.tag_commit_sha,'d7215b0ad3fc2c562e8317fd9bca058fd4863804');
  assert.notEqual(gitBlobSha(Buffer.from('LICENSE')),gitBlobSha(Buffer.from('LICENCE')));
});
test('release admission, wrong binary license, wrong tag, notices and SHA pins all fail closed',()=>{
  for(const [which,key,to] of [
    ['root','release_admitted',true],
    ['root','installer_signing_authorized',true],
    ['root','stage_binary_distribution_authorized',true],
    ['yt','license','Unlicense'],
    ['yt','source_license','GPL-3.0-or-later'],
    ['yt','tag_commit_sha','0'.repeat(40)],
    ['yt','sha256','0'.repeat(64)],
    ['yt','license_notice','https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/LICENSE'],
    ['yt','third_party_license_blob_sha1','0'.repeat(40)],
    ['yt','license_status','APPROVED'],
    ['q','license','Unlicense'],
    ['q','license_blob_sha1','0'.repeat(40)],
    ['q','tag_commit_sha','0'.repeat(40)],
    ['mux','license','MIT']
  ]){
    const a=allow();
    const item=which==='root'?a:which==='yt'?a.components[3]:which==='q'?a.components[4]:a.components[2];
    item[key]=to;
    assert.throws(()=>validateMetadata(a),/VIDEO_TOOLS_01Q_HOLD/,
      which+'.'+key+' must never self-approve binary release');
  }
});
const source=()=>({
  ytReadme:Buffer.from('While yt-dlp is licensed under the [Unlicense](LICENSE), many release files differ.\nMost notably, the PyInstaller-bundled executables include GPLv3+ licensed code, and as such the combined work is licensed under [GPLv3+].\nSee THIRD_PARTY_LICENSES.txt.'),
  ytThird:Buffer.from('THIRD-PARTY LICENSES\n\nThis file aggregates license texts of third-party components included with the yt-dlp PyInstaller-bundled executables.'),
  ytLicense:Buffer.from('This is free and unencumbered software released into the public domain.'),
  qjsLicense:Buffer.from('The MIT License (MIT)\nThe above copyright notice and this permission notice shall be included in all copies.')
});
test('upstream licensing semantics and third-party text remain present in source',()=>{
  validateText(source());
  const s=source();s.ytReadme=Buffer.from(s.ytReadme.toString('utf8').replace('GPLv3+','MIT'));
  assert.throws(()=>validateText(s),/VIDEO_TOOLS_01Q_HOLD/);
  const t=source();t.ytThird=Buffer.from('THIRD-PARTY LICENSES\nNot the actual bundled notices');
  assert.throws(()=>validateText(t),/VIDEO_TOOLS_01Q_HOLD/);
  const u=source();u.qjsLicense=Buffer.from('The MIT License (MIT)\nNotice omitted');
  assert.throws(()=>validateText(u),/VIDEO_TOOLS_01Q_HOLD/);
});
test('exact-HEAD pinned upstream checkout + one-day NON-APPROVAL report are mandatory',()=>{
  const s=wf();
  for(const token of [
    "      - '.github/scripts/video-tools-01q-executable-license-evidence.cjs'",
    "      - '.github/scripts/video-tools-01q-executable-license-evidence.test.cjs'",
    "      - 'docs/release/video-tools-01q-binary-license-review.md'",
    "node --test .github/scripts/video-tools-01q-executable-license-evidence.test.cjs",
    'repository: yt-dlp/yt-dlp',
    'ref: f264a6cbd1f630894da0f368df00f8aae50140e0',
    'repository: quickjs-ng/quickjs',
    'ref: d7215b0ad3fc2c562e8317fd9bca058fd4863804',
    'persist-credentials: false',
    'node .github/scripts/video-tools-01q-executable-license-evidence.cjs',
    'VIDEO_TOOLS_01Q_SOURCE_LICENSE_PASS',
    'video-tools-01q-license-HOLD.json',
    'retention-days: 1',
    'permissions:\n  contents: read'
  ])assert.ok(s.includes(token),'VIDEO_TOOLS_01Q_HOLD: source evidence workflow omitted '+token);
  assert.ok(!/workflow_dispatch:|^\s+push:|contents:\s*write|gh release create|publish_beta/m.test(s),
    'VIDEO_TOOLS_01Q_HOLD: legal evidence workflow can not release binaries');
});
