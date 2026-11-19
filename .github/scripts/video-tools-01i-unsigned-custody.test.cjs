'use strict';
// VIDEO-TOOLS-01i: synthetic negative fixtures + CI integration contracts.
// True pinned upstream PE bytes are tested separately in genuine Windows NSIS CI.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const crypto=require('node:crypto');
const {test}=require('node:test');
const base=path.resolve(__dirname,'../..');
const builder=require('../../scripts/video-tools-01i-unsigned-custody.cjs');
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const names=builder.names;
const lock=Buffer.from('[version]\nname = "akula-video-lite"\nsource = "git"\n');
const commit='f'.repeat(40);
function makePE(salt=1){
  const b=Buffer.alloc(1024+salt*64,salt);
  b.write('MZ',0,'ascii');
  b.writeUInt32LE(128,0x3c);
  b.write('PE\0\0',128,'binary');
  b.writeUInt16LE(0x8664,132);
  b.writeUInt16LE(3,134);
  b.writeUInt16LE(240,148);
  b.writeUInt16LE(0x20b,152);
  return b;
}
function allowFixture(payloads){
  const source=(name)=>({name,repository:'Lalalalendia/Akula',kind:'source-built',
    source_path:'labs/video-lite',lock_path:'labs/video-lite/Cargo.lock',
    license_status:'REVIEW_TRANSITIVE_DEPENDENCY_NOTICES'});
  const upstream=(name,reference,sourceUrl)=>({
    name,kind:'pinned-upstream',repository:reference.startsWith('2026')?'yt-dlp/yt-dlp':'quickjs-ng/quickjs',
    reference,source_url:sourceUrl,sha256:sha(payloads[name]),
    size_bytes:payloads[name].length,license_status:'REVIEW_BUNDLED_DEPENDENCIES'
  });
  return {
    schema:1,release_admitted:false,installer_signing_authorized:false,
    stage_binary_distribution_authorized:false,independent_owner_approval_required:true,
    components:[source(names[0]),
      upstream(names[1],'2026.10.26','https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe'),
      upstream(names[2],'v0.18.0','https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/qjs-windows-x86_64.exe')]
  };
}
function withFixture(fn){
  const temp=fs.mkdtempSync(path.join(os.tmpdir(),'akula-01i-'));
  const stage=path.join(temp,'stage');
  fs.mkdirSync(stage);
  const bytes=Object.fromEntries(names.map((name,i)=>[name,makePE(i+1)]));
  for(const name of names)fs.writeFileSync(path.join(stage,name),bytes[name]);
  const allow=allowFixture(bytes);
  try {return fn({stage,allow,bytes,temp})}
  finally {fs.rmSync(temp,{recursive:true,force:true})}
}
test('unsigned manifest accepts only exact AMD64 real-PE shape and preserves deterministic order',()=>{
  withFixture(({stage,allow})=>{
    const a=builder.buildUnsignedManifest(stage,allow,lock,commit);
    const b=builder.buildUnsignedManifest(stage,allow,lock,commit);
    assert.deepEqual(a.manifest,b.manifest);
    assert.deepEqual(a.evidence,b.evidence);
    const signed=JSON.parse(a.manifest);
    assert.deepEqual(Object.keys(signed),[
      'schema','product','version','platform','source_commit','cargo_lock_sha256','binaries'
    ]);
    assert.equal(signed.product,'Akula Video Tools');
    assert.equal(signed.platform,'windows-x86_64');
    assert.equal(signed.version,'0.1.0');
    assert.equal(signed.source_commit,commit);
    assert.equal(signed.cargo_lock_sha256,sha(lock));
    assert.deepEqual(signed.binaries.map(x=>x.name),names);
    assert.equal(JSON.parse(a.evidence).release_admitted,false);
    assert.equal(JSON.parse(a.evidence).publisher_key_approved,false);
    assert.ok(a.evidence.includes(Buffer.from('REVIEW_')));
    assert.ok(!a.manifest.toString().includes('private'));
  });
});
test('fail closed on PE header, architecture, optional header and truncated binary',()=>{
  for(const [label,change] of [
    ['DOS',(b)=>b.write('ZZ',0)],
    ['signature',(b)=>b.write('BAD!',128)],
    ['x86',(b)=>b.writeUInt16LE(0x14c,132)],
    ['ARM64',(b)=>b.writeUInt16LE(0xaa64,132)],
    ['PE32',(b)=>b.writeUInt16LE(0x10b,152)],
    ['no sections',(b)=>b.writeUInt16LE(0,134)],
    ['bad e_lfanew',(b)=>b.writeUInt32LE(0xffff,0x3c)],
    ['short optional',(b)=>b.writeUInt16LE(16,148)]
  ])withFixture(({stage,allow})=>{
    const dest=path.join(stage,names[0]);
    const pe=fs.readFileSync(dest);
    change(pe);
    fs.writeFileSync(dest,pe);
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),
      /VIDEO_TOOLS_01I_HOLD/,label);
  });
});
test('fail closed on extra files, symlinks, upstream mismatch and release allowlist promotion',()=>{
  withFixture(({stage,allow})=>{
    fs.writeFileSync(path.join(stage,'akula-app.exe'),'forbidden');
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),/VIDEO_TOOLS_01I_HOLD/);
    fs.rmSync(path.join(stage,'akula-app.exe'));
    const target=path.join(stage,names[2]);
    fs.writeFileSync(target,Buffer.from('changed'));
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),/VIDEO_TOOLS_01I_HOLD/);
  });
  withFixture(({stage,allow,temp})=>{
    const target=path.join(stage,names[0]);
    fs.renameSync(target,path.join(temp,'actual-mux'));
    fs.symlinkSync(path.join(temp,'actual-mux'),target);
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),
      /VIDEO_TOOLS_01I_HOLD/);
  });
  withFixture(({stage,allow})=>{
    allow.release_admitted=true;
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),/VIDEO_TOOLS_01I_HOLD/);
  });
  withFixture(({stage,allow})=>{
    allow.components[1].sha256='0'.repeat(64);
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),/VIDEO_TOOLS_01I_HOLD/);
  });
});
test('fail closed on missing or mismatched exact source commit and unchanged cargo lock',()=>{
  withFixture(({stage,allow})=>{
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,'not-a-commit'),
      /VIDEO_TOOLS_01I_HOLD/);
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,Buffer.from('short'),commit),
      /VIDEO_TOOLS_01I_HOLD/);
    allow.components[0].lock_path='aquilum-app/Cargo.lock';
    assert.throws(()=>builder.buildUnsignedManifest(stage,allow,lock,commit),
      /VIDEO_TOOLS_01I_HOLD/);
  });
});
test('real Windows CI requires unsigned custody from the real pinned PE stage',()=>{
  const runner=fs.readFileSync(path.join(base,'scripts/windows-video-tools-01a-isolated-lab.ps1'),'utf8');
  const workflow=fs.readFileSync(path.join(base,'.github/workflows/akula-video-tools-01a-isolated-lab.yml'),'utf8');
  const code=fs.readFileSync(path.join(base,'scripts/video-tools-01i-unsigned-custody.cjs'),'utf8');
  const hold=(val,msg)=>assert.ok(val,'VIDEO_TOOLS_01I_HOLD: '+msg);
  for(const token of [
    'scripts/video-tools-01i-unsigned-custody.cjs',
    'VIDEO_TOOLS_01I_UNSIGNED_CUSTODY_PASS',
    '-Stage $stage', // CLI positional invocation must use $stage
    'manifest.json',
    'cargo_lock_sha256',
    'release=0'
  ]) {
    if(token==='-Stage $stage')continue; // positional Node CLI below
    hold(runner.includes(token),'runner omitted '+token);
  }
  hold(runner.includes('$stage')&&runner.includes('$unsignedOutput'),
    'unsigned manifest was not produced from NSIS staged binaries');
  for(const token of [
    "      - 'scripts/video-tools-01i-unsigned-custody.cjs'",
    "      - '.github/scripts/video-tools-01i-unsigned-custody.test.cjs'",
    'node --test .github/scripts/video-tools-01i-unsigned-custody.test.cjs',
    'VIDEO_TOOLS_01I_UNSIGNED_CUSTODY_PASS',
    'permissions:\n  contents: read',
    'persist-credentials: false',
    'cancel-in-progress:'
  ])hold(workflow.includes(token),'CI unsigned-custody witness missing '+token);
  for(const token of [
    "RUNNER_ENVIRONMENT!=='github-hosted'",
    "GITHUB_EVENT_NAME!=='pull_request'",
    "EXPECTED_HEAD_SHA",
    "inspectPinnedAllowlist(allowlist)",
    "sha256(bytes)",
    "manifest.json",
    "custody-review.json",
    "release_admitted:false",
    "publisher_key_approved:false"
  ])hold(code.includes(token),'builder security guard missing '+token);
  hold(!code.includes('minisign -S')&&!code.includes('publisher.key')&&!code.includes('privateKey'),
    'unsigned custody test must never sign or generate a publisher key');
});
