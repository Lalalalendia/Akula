'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const {main, inspect, decodeManifest, boundedManifest, PLATFORMS} = require('./verify-release-candidate.cjs');

const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const COMMIT = 'a'.repeat(40);
const LOCK = 'b'.repeat(64);
const VERSION = '0.1.0';
const NAMES = [
  'Akula_' + VERSION + '_x64-setup.exe',
  'akula-app_' + VERSION + '_x86_64.AppImage',
  'Akula_' + VERSION + '_amd64.deb',
  'Akula-' + VERSION + '-1.x86_64.rpm',
];
function command(bin, args) {
  const out = cp.spawnSync(bin, args, {encoding:'utf8', timeout:30000});
  assert.equal(out.status, 0, bin + ' ' + args.join(' ') + ': ' + out.stderr);
}
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'akula-trust-lab-'));
  const artifacts = {};
  for (const [i, platform] of PLATFORMS.entries()) {
    const filename = NAMES[i];
    const bytes = Buffer.from('AKULA-ISOLATED-FIXTURE-' + i);
    fs.writeFileSync(path.join(dir, filename), bytes);
    artifacts[platform] = {filename, sha256:hash(bytes), size:bytes.length};
  }
  const manifest = {schema:1, product:'Akula', version:VERSION,
    source_commit:COMMIT, cargo_lock_sha256:LOCK, artifacts};
  const manifestFile = path.join(dir,'manifest.json');
  const signatureFile = path.join(dir,'manifest.minisig');
  const publicFile = path.join(dir,'lab.pub');
  const privateFile = path.join(dir,'lab.key');
  function write() { fs.writeFileSync(manifestFile, JSON.stringify(manifest)); }
  function key() { command('minisign',['-G','-W','-s',privateFile,'-p',publicFile]); }
  function sign() { command('minisign',['-S','-s',privateFile,'-m',manifestFile,'-x',signatureFile,'-q']); }
  write();
  return {dir, manifest, manifestFile, signatureFile, publicFile, privateFile, write, key, sign,
    args:()=>[manifestFile,signatureFile,publicFile,dir,VERSION,COMMIT,LOCK]};
}
const identity = {version:VERSION,source_commit:COMMIT,cargo_lock_sha256:LOCK};

test('independent ephemeral Minisign key authenticates exact candidate bytes', t => {
  const f=fixture(); t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  f.key(); f.sign();
  assert.doesNotThrow(()=>main(f.args()));
  // The attacker does not get to increase package bytes, even with a signed manifest.
  fs.appendFileSync(path.join(f.dir,NAMES[0]),'X');
  assert.throws(()=>main(f.args()),/file type\/size mismatch/);
  // Changing signed metadata without resigning cannot override publisher identity.
  const original = JSON.parse(fs.readFileSync(f.manifestFile,'utf8'));
  fs.writeFileSync(f.manifestFile, JSON.stringify({...original,version:'0.2.0'}));
  assert.throws(()=>main(f.args()),/publisher signature verification failed/);
});
test('wrong trusted key, source commit, version and Cargo.lock digest refuse access',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  f.key();f.sign();
  const second=path.join(f.dir,'second.pub');
  command('minisign',['-G','-W','-s',path.join(f.dir,'second.key'),'-p',second]);
  const wrongKey=f.args();wrongKey[2]=second;
  assert.throws(()=>main(wrongKey),/publisher signature verification failed/);
  const wrongCommit=f.args();wrongCommit[5]='f'.repeat(40);
  assert.throws(()=>main(wrongCommit),/unexpected release identity/);
  const wrongVersion=f.args();wrongVersion[4]='0.2.0';
  assert.throws(()=>main(wrongVersion),/unexpected release identity/);
  const wrongLock=f.args();wrongLock[6]='e'.repeat(64);
  assert.throws(()=>main(wrongLock),/unexpected release identity/);
});
test('rejects traversal, duplicate, wrong platform and incomplete platform matrix',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  assert.equal(inspect(f.manifest,f.dir,identity),true);
  const p=PLATFORMS[0],q=PLATFORMS[1],first=NAMES[0],second=NAMES[1];
  f.manifest.artifacts[p].filename='../escaped';
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/unsafe artifact filename/);
  f.manifest.artifacts[p].filename=second;
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/duplicate artifact filename/);
  f.manifest.artifacts[q].filename=first;
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/platform filename mismatch/);
  f.manifest.artifacts[p].filename=first;
  f.manifest.artifacts[q].filename=second;
  f.manifest.artifacts.bad={...f.manifest.artifacts[p]};
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/missing or unknown fields/);
  delete f.manifest.artifacts.bad;
  delete f.manifest.artifacts[q];
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/missing or unknown fields/);
});
test('same-length byte tampering, upstream product and a leaf symlink are refused',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  f.manifest.product='Aquilum';
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/wrong product/);
  f.manifest.product='Akula';
  const filename=path.join(f.dir,NAMES[0]);
  const bytes=fs.readFileSync(filename);
  fs.writeFileSync(filename,Buffer.concat([Buffer.from('X'),bytes.subarray(1)]));
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/artifact digest mismatch/);
  fs.writeFileSync(filename,bytes);
  fs.renameSync(filename,filename+'.copy');
  fs.symlinkSync(filename+'.copy',filename);
  assert.throws(()=>inspect(f.manifest,f.dir,identity),/ELOOP|not a regular file/);
});
test('canonical signed JSON refuses duplicate keys and differently encoded JSON',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  f.key();
  const text=fs.readFileSync(f.manifestFile,'utf8');
  const duplicate=text.replace('"product":"Akula",','"product":"Akula","product":"Akula",');
  fs.writeFileSync(f.manifestFile,duplicate);
  f.sign();
  assert.throws(()=>main(f.args()),/noncanonical manifest JSON/);
  assert.throws(()=>decodeManifest(Buffer.from(' { "product": "Akula" }')),/noncanonical manifest JSON/);
});
test('oversized manifest is rejected before signature verification and allocation',t=>{
  const f=fixture();t.after(()=>fs.rmSync(f.dir,{recursive:true,force:true}));
  fs.writeFileSync(f.manifestFile,Buffer.alloc(65537,32));
  assert.throws(()=>boundedManifest(f.manifestFile),/manifest size invalid/);
  assert.throws(()=>main(f.args()),/manifest size invalid/);
});
