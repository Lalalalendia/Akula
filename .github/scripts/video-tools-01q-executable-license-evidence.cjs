'use strict';
// VIDEO-TOOLS-01q — exact upstream source/license evidence ONLY.
// Never a legal approval, installer payload, publisher signing or release gate.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const cp=require('node:child_process');
const NAMES=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
const YT='f264a6cbd1f630894da0f368df00f8aae50140e0';
const QJS='d7215b0ad3fc2c562e8317fd9bca058fd4863804';
const YT_ASSET='f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d';
const QJS_ASSET='ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7';
const BLOB={
  ytReadme:'750fae655ea0d5221bf49901508deb44681f3db0',
  ytThird:'88f9872c66b456703e91ca32db4041e1ec47258d',
  ytLicense:'68a49daad8ff7e35068f2b7a97d643aab440eaec',
  qjsLicense:'fae657320cdf957cab10654ac39990fdb20e97ef'
};
function hold(why){throw Error('VIDEO_TOOLS_01Q_HOLD: '+why)}
function gitBlobSha(bytes){
  assert.ok(Buffer.isBuffer(bytes));
  return crypto.createHash('sha1').update(Buffer.from('blob '+bytes.length+'\0')).update(bytes).digest('hex');
}
function readSource(root,rel){
  const base=fs.realpathSync(root);
  const file=path.join(base,rel);
  const actual=fs.realpathSync(file);
  if(actual===base||!actual.startsWith(base+path.sep))hold('license file escapes pinned source tree');
  const stat=fs.lstatSync(file);
  if(!stat.isFile()||stat.isSymbolicLink()||stat.size<16||stat.size>1024*1024)
    hold('license document missing, linked, empty or oversized');
  const bytes=fs.readFileSync(file);
  if(bytes.length!==stat.size)hold('license text size changed while reading');
  return bytes;
}
function validateMetadata(allow){
  if(allow?.schema!==1||allow.release_admitted!==false||
     allow.installer_signing_authorized!==false||
     allow.stage_binary_distribution_authorized!==false||
     allow.independent_owner_approval_required!==true||
     JSON.stringify(allow.components?.map(x=>x.name))!==JSON.stringify(NAMES)||
     allow.components.some(x=>!String(x.license_status).startsWith('REVIEW_')))hold('release HOLD or component identity changed');
  const mux=allow.components[2],yt=allow.components[3],q=allow.components[4];
  if(mux.kind!=='source-built'||mux.license!=='AGPL-3.0-only'||
     mux.license_status!=='REVIEW_TRANSITIVE_DEPENDENCY_NOTICES')hold('Rust mux license provenance changed');
  if(yt.kind!=='pinned-upstream'||yt.repository!=='yt-dlp/yt-dlp'||
     yt.reference!=='2026.10.26'||yt.tag_commit_sha!==YT||
     yt.source_url!=='https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe'||
     yt.sha256!==YT_ASSET||yt.size_bytes!==23575744||
     yt.license!=='GPL-3.0-or-later'||yt.source_license!=='Unlicense'||
     yt.license_status!=='REVIEW_BUNDLED_DEPENDENCIES'||
     yt.license_notice!=='https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/THIRD_PARTY_LICENSES.txt'||
     yt.source_license_notice!=='https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/LICENSE'||
     yt.combined_license_evidence!=='https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/README.md#licensing'||
     yt.readme_blob_sha1!==BLOB.ytReadme||
     yt.third_party_license_blob_sha1!==BLOB.ytThird||
     yt.source_license_blob_sha1!==BLOB.ytLicense)hold('yt-dlp source-vs-PyInstaller license or immutable asset pin changed');
  if(q.kind!=='pinned-upstream'||q.repository!=='quickjs-ng/quickjs'||
     q.reference!=='v0.18.0'||q.tag_commit_sha!==QJS||
     q.source_url!=='https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/qjs-windows-x86_64.exe'||
     q.sha256!==QJS_ASSET||q.size_bytes!==1873152||
     q.license!=='MIT'||q.license_status!=='REVIEW_NOTICE_PACKAGING'||
     q.license_notice!=='https://github.com/quickjs-ng/quickjs/blob/v0.18.0/LICENSE'||
     q.license_blob_sha1!==BLOB.qjsLicense)hold('QuickJS exact tagged MIT release evidence changed');
  return {mux,yt,q};
}
function validateText(doc){
  const readme=doc.ytReadme.toString('utf8');
  const third=doc.ytThird.toString('utf8');
  const ytsrc=doc.ytLicense.toString('utf8');
  const qjs=doc.qjsLicense.toString('utf8');
  if(!readme.includes('PyInstaller-bundled executables include GPLv3+ licensed code')||
     !readme.includes('combined work is licensed under')||
     !readme.includes('THIRD_PARTY_LICENSES.txt')||
     !readme.includes('While yt-dlp is licensed under the [Unlicense]'))
    hold('tagged yt-dlp README no longer supports binary GPLv3+ distinction');
  if(!third.startsWith('THIRD-PARTY LICENSES')||
     !third.includes('This file aggregates license texts of third-party components included with the yt-dlp PyInstaller-bundled executables.'))
    hold('yt-dlp bundled third-party notices missing');
  if(!ytsrc.includes('This is free and unencumbered software released into the public domain.'))
    hold('yt-dlp source Unlicense text missing');
  if(!qjs.startsWith('The MIT License (MIT)')||
     !qjs.includes('The above copyright notice and this permission notice shall be included'))
    hold('QuickJS tagged MIT license attribution terms missing');
}
function evidence(allow,doc){
  const {mux,yt,q}=validateMetadata(allow);
  validateText(doc);
  for(const [key,expected] of Object.entries(BLOB)){
    if(gitBlobSha(doc[key])!==expected)hold('exact tagged upstream license blob mismatch: '+key);
  }
  return {
    schema:1,title:'VIDEO-TOOLS-01q executable license evidence — NON-APPROVAL',
    scope:'optional three-runtime Windows V1',release_admitted:false,
    publisher_key_approved:false,legal_approval:false,license_text_review_complete:false,
    binary_contents_sbom_verified:false,third_party_notices_packaged:false,
    source_built_mux:{name:mux.name,source_license:mux.license,review_status:mux.license_status,
      actual_linked_binary_sbom_verified:false},
    pinned_executables:[
      {name:yt.name,tag:yt.reference,tag_commit_sha:YT,release_asset_sha256:YT_ASSET,
       size_bytes:yt.size_bytes,source_license:'Unlicense',windows_binary_combined_license:'GPL-3.0-or-later',
       upstream_readme_blob_sha1:BLOB.ytReadme,upstream_third_party_license_blob_sha1:BLOB.ytThird,
       upstream_source_license_blob_sha1:BLOB.ytLicense,review_status:yt.license_status},
      {name:q.name,tag:q.reference,tag_commit_sha:QJS,release_asset_sha256:QJS_ASSET,
       size_bytes:q.size_bytes,source_license:'MIT',upstream_license_blob_sha1:BLOB.qjsLicense,
       review_status:q.license_status}
    ],
    pending_decisions:[
      'Independent human review of GPLv3+ obligations and yt-dlp PyInstaller bundle actual constituents/source availability',
      'Deliver version-specific THIRD_PARTY_LICENSES and necessary attribution/source notices with package; placement not yet designed',
      'Review QuickJS tagged MIT copyright and permission notice placement',
      'Review AGPL Rust mux source/lock and transitive third-party notices; existing Cargo graph is not linked binary SBOM',
      'Owner signs exact approved package and manifest; Windows 10/11 end-user admission remains blocked'
    ],
    notes:'Pinned source license text Git blob digests are checked. Release asset SHA and sizes are pinned to recorded GitHub release metadata but EXE bytes are not downloaded or unpacked in this source-only report.'
  };
}
function gitHead(dir){
  return cp.execFileSync('git',['-C',dir,'rev-parse','HEAD'],
    {encoding:'utf8',stdio:['ignore','pipe','ignore'],timeout:15000}).trim();
}
function main(){
  if(process.argv.length!==7)hold('expected allowlist yt-source quickjs-source expected-head report');
  const [,,allowPath,ytRoot,qRoot,head,out]=process.argv;
  if(process.env.GITHUB_ACTIONS!=='true'||process.env.GITHUB_EVENT_NAME!=='pull_request'||
     !/^[0-9a-f]{40}$/.test(head)||head!==process.env.EXPECTED_HEAD_SHA||
     gitHead(process.env.GITHUB_WORKSPACE||'.')!==head||
     gitHead(ytRoot)!==YT||gitHead(qRoot)!==QJS)hold('CI source/tag identity not approved');
  const temp=fs.realpathSync(process.env.RUNNER_TEMP||'.');
  const dest=path.resolve(out);
  if(!dest.startsWith(temp+path.sep)||path.basename(dest)!=='video-tools-01q-license-HOLD.json'||
     fs.existsSync(dest))hold('report outside disposable runner or already exists');
  const a=JSON.parse(fs.readFileSync(allowPath,'utf8'));
  const doc={
    ytReadme:readSource(ytRoot,'README.md'),
    ytThird:readSource(ytRoot,'THIRD_PARTY_LICENSES.txt'),
    ytLicense:readSource(ytRoot,'LICENSE'),
    qjsLicense:readSource(qRoot,'LICENSE')
  };
  const result=evidence(a,doc);
  result.exact_akula_head=head;
  fs.writeFileSync(dest,JSON.stringify(result,null,2)+'\n',{flag:'wx'});
  console.log('VIDEO_TOOLS_01Q_SOURCE_LICENSE_PASS pinned_tags=2 verified_texts=4 binary_gpl3plus=1 legal_approval=0 release=0');
}
if(require.main===module){
  try{main()}catch(e){console.error(String(e.message||e));process.exit(3)}
}
module.exports={validateMetadata,validateText,gitBlobSha,readSource,evidence};
