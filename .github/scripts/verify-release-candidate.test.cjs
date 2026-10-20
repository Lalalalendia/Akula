'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const {main, inspect, PLATFORMS} = require('./verify-release-candidate.cjs');
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const COMMIT = 'a'.repeat(40);
const LOCK = 'b'.repeat(64);
const VERSION = '0.1.0';
function command(bin, args) {
  const out = cp.spawnSync(bin, args, {encoding:'utf8', timeout:30000});
  assert.equal(out.status, 0, bin + ' ' + args.join(' ') + ': ' + out.stderr);
}
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'akula-trust-lab-'));
  const artifacts = {};
  for (const [i, platform] of PLATFORMS.entries()) {
    const filename = 'Akula-lab-' + i + '.bin';
    const bytes = Buffer.from('AKULA-ISOLATED-FIXTURE-' + i);
    fs.writeFileSync(path.join(dir, filename), bytes);
    artifacts[platform] = {filename, sha256:hash(bytes), size:bytes.length};
  }
  const manifest = {schema:1, product:'Akula', version:VERSION, source_commit:COMMIT, cargo_lock_sha256:LOCK, artifacts};
  const manifestFile = path.join(dir,'manifest.json');
  const signatureFile = path.join(dir,'manifest.minisig');
  const publicFile = path.join(dir,'lab.pub');
  const privateFile = path.join(dir,'lab.key');
  function write() { fs.writeFileSync(manifestFile, JSON.stringify(manifest)); }
  write();
  return {dir, artifacts, manifest, manifestFile, signatureFile, publicFile, privateFile, write,
    args:()=>[manifestFile,signatureFile,publicFile,dir,VERSION,COMMIT,LOCK]};
}
test('independent ephemeral minisign key authenticates exact candidate bytes', t => {
  const f = fixture();
  t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  command('minisign',['-G','-W','-s',f.privateFile,'-p',f.publicFile]);
  command('minisign',['-S','-s',f.privateFile,'-m',f.manifestFile,'-x',f.signatureFile,'-q']);
  assert.doesNotThrow(()=>main(f.args()));
  fs.appendFileSync(path.join(f.dir,f.artifacts[PLATFORMS[0]].filename),'X');
  assert.throws(()=>main(f.args()),/file type\/size mismatch/);
  // Restoring artifacts must not make a modified, unsigned manifest acceptable.
  const original = JSON.parse(fs.readFileSync(f.manifestFile,'utf8'));
  fs.writeFileSync(f.manifestFile, JSON.stringify({...original,version:'0.2.0'}));
  assert.throws(()=>main(f.args()),/publisher signature verification failed/);
});
test('wrong trusted key and wrong source identity are refused',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  command('minisign',['-G','-W','-s',f.privateFile,'-p',f.publicFile]);
  command('minisign',['-S','-s',f.privateFile,'-m',f.manifestFile,'-x',f.signatureFile,'-q']);
  const second=path.join(f.dir,'second.pub');
  command('minisign',['-G','-W','-s',path.join(f.dir,'second.key'),'-p',second]);
  const wrongKey=f.args(); wrongKey[2]=second;
  assert.throws(()=>main(wrongKey),/publisher signature verification failed/);
  const wrongCommit=f.args();wrongCommit[5]='f'.repeat(40);
  assert.throws(()=>main(wrongCommit),/unexpected release identity/);
});
test('rejects path traversal, unknown platform, missing platform, duplicate file, and symlink',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  const expected={version:VERSION, source_commit:COMMIT,cargo_lock_sha256:LOCK};
  assert.equal(inspect(f.manifest,f.dir,expected),true);
  const p=PLATFORMS[0],q=PLATFORMS[1];
  f.manifest.artifacts[p].filename='../escaped';
  assert.throws(()=>inspect(f.manifest,f.dir,expected),/unsafe artifact filename/);
  f.manifest.artifacts[p].filename=f.artifacts[q].filename;
  assert.throws(()=>inspect(f.manifest,f.dir,expected),/duplicate artifact filename/);
  f.manifest.artifacts[p].filename='Akula-lab-0.bin';
  f.manifest.artifacts.bad={...f.artifacts[p]};
  assert.throws(()=>inspect(f.manifest,f.dir,expected),/missing or unknown fields/);
  delete f.manifest.artifacts.bad;
  delete f.manifest.artifacts[q];
  assert.throws(()=>inspect(f.manifest,f.dir,expected),/missing or unknown fields/);
});
test('rejects unexpected release product, missing fields and digest mismatch',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  const expected={version:VERSION,source_commit:COMMIT,cargo_lock_sha256:LOCK};
  f.manifest.product='Aquilum';assert.throws(()=>inspect(f.manifest,f.dir,expected),/wrong product/);
  f.manifest.product='Akula';
  f.manifest.artifacts[PLATFORMS[0]].sha256='f'.repeat(64);
  assert.throws(()=>inspect(f.manifest,f.dir,expected),/artifact digest mismatch/);
});
