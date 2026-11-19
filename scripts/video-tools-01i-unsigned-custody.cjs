'use strict';
// VIDEO-TOOLS-01i — deterministic UNSIGNED custody manifest from real Windows PEs.
// CI evidence only; never invokes Minisign, installs files, approves releases or ships.
const fs=require('node:fs');
const path=require('node:path');
const cp=require('node:child_process');
const crypto=require('node:crypto');

const names=Object.freeze(['akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe']);
const version='0.1.0';
const expectedProduct='Akula Video Tools';
const expectedPlatform='windows-x86_64';
function hold(why){throw Error('VIDEO_TOOLS_01I_HOLD: '+why)}
const hex=(value,len)=>typeof value==='string'&&new RegExp('^[a-f0-9]{'+len+'}$').test(value);
const sha256=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
function regular(file){
  const info=fs.lstatSync(file);
  if(info.isSymbolicLink()||!info.isFile()||info.size<64||info.size>128*1024*1024){
    hold('missing, aliased, empty, or oversized staged executable');
  }
  return info;
}
function assertAmd64PE(file){
  const stat=regular(file);
  const fd=fs.openSync(file,fs.constants.O_RDONLY);
  try {
    const header=Buffer.alloc(Math.min(4096,stat.size));
    const bytes=fs.readSync(fd,header,0,header.length,0);
    if(bytes<512 || header.toString('binary',0,2)!=='MZ')
      hold('staged executable has no DOS/PE header');
    const offset=header.readUInt32LE(0x3c);
    if(offset<64||offset>header.length-32)
      hold('malformed or unbounded PE header offset');
    if(header.toString('binary',offset,offset+4)!=='PE\u0000\u0000')
      hold('staged executable has no PE signature');
    const machine=header.readUInt16LE(offset+4);
    const sections=header.readUInt16LE(offset+6);
    const optionalSize=header.readUInt16LE(offset+20);
    const magic=header.readUInt16LE(offset+24);
    if(machine!==0x8664||magic!==0x20b||sections<1||sections>96||optionalSize<112)
      hold('staged executable is not a bounded AMD64 PE32+ image');
    if(offset+24+optionalSize>stat.size)
      hold('truncated optional PE header');
    return {machine:'AMD64',optional_header:'PE32+',sections};
  } finally { fs.closeSync(fd) }
}
function exactThree(stage){
  if(!fs.lstatSync(stage).isDirectory()||fs.lstatSync(stage).isSymbolicLink())
    hold('staging root is not a real directory');
  const all=fs.readdirSync(stage);
  if(all.length!==names.length || !names.every(n=>all.includes(n)))
    hold('unsigned staging must have exactly three named executable files');
}
function inspectPinnedAllowlist(allowlist){
  if(allowlist.schema!==1 ||
    allowlist.release_admitted!==false ||
    allowlist.installer_signing_authorized!==false ||
    allowlist.stage_binary_distribution_authorized!==false ||
    allowlist.independent_owner_approval_required!==true ||
    !Array.isArray(allowlist.components))hold('source allowlist was modified or promoted');
  const result={};
  for(const name of names){
    const entries=allowlist.components.filter(x=>x.name===name);
    if(entries.length!==1)hold('ambiguous pinned source identity '+name);
    const item=entries[0];
    if(name==='akula-video-mux.exe'){
      if(item.kind!=='source-built'||item.repository!=='Lalalalendia/Akula'||
        item.lock_path!=='labs/video-lite/Cargo.lock'||
        item.source_path!=='labs/video-lite' ||
        item.license_status!=='REVIEW_TRANSITIVE_DEPENDENCY_NOTICES')
        hold('source-built Rust mux provenance changed');
    } else if(item.kind!=='pinned-upstream'||
      !hex(item.sha256,64)||!Number.isSafeInteger(item.size_bytes)||
      item.size_bytes<512||!/^REVIEW_/.test(item.license_status)||
      !String(item.source_url).startsWith('https://github.com/')){
      hold('unapproved or unpinned upstream component '+name);
    }
    result[name]=item;
  }
  if(result['akula-yt-dlp.exe'].reference!=='2026.10.26'||
     result['akula-quickjs.exe'].reference!=='v0.18.0')
    hold('external provenance revision drifted without owner review');
  return result;
}
function buildUnsignedManifest(stage,allowlist,lockBytes,sourceCommit){
  if(!hex(sourceCommit,40)||!Buffer.isBuffer(lockBytes)||lockBytes.length<32)
    hold('untrusted repository revision or Cargo.lock contents');
  exactThree(stage);
  const entries=inspectPinnedAllowlist(allowlist);
  const binaries=[];
  const custody=[];
  for(const name of names){
    const file=path.join(stage,name);
    const pe=assertAmd64PE(file);
    const bytes=fs.readFileSync(file);
    const hash=sha256(bytes);
    const src=entries[name];
    if(src.kind==='pinned-upstream' && (hash!==src.sha256||bytes.length!==src.size_bytes))
      hold('upstream source byte pin mismatch '+name);
    binaries.push({name,sha256:hash,size:bytes.length});
    custody.push({
      name,sha256:hash,size:bytes.length,
      pe_machine:pe.machine,pe_format:pe.optional_header,
      origin:src.kind,repository:src.repository,reference:src.reference,
      source_url:src.source_url||null,
      license_status:src.license_status
    });
  }
  const lockHash=sha256(lockBytes);
  const manifest={
    schema:1,product:expectedProduct,version,platform:expectedPlatform,
    source_commit:sourceCommit,cargo_lock_sha256:lockHash,binaries
  };
  // This is a custody review document OUTSIDE the eventual installed root.
  // No signature or test key is generated here; no release can be approved.
  const evidence={
    schema:1,purpose:'VIDEO-TOOLS-01i PR-only unsigned custody witness',
    release_admitted:false,publisher_key_approved:false,
    source_commit:sourceCommit,cargo_lock_sha256:lockHash,
    components:custody
  };
  return {
    manifest:Buffer.from(JSON.stringify(manifest)+'\n','utf8'),
    evidence:Buffer.from(JSON.stringify(evidence)+'\n','utf8')
  };
}
function under(target,base){
  const rel=path.relative(fs.realpathSync(base),fs.realpathSync(target));
  return rel!=='' && rel!=='..'&&!rel.startsWith('..'+path.sep)&&!path.isAbsolute(rel);
}
function cli(){
  if(process.argv.length!==4)hold('expected exactly stage and unique output directory');
  if(process.env.GITHUB_ACTIONS!=='true'||process.env.RUNNER_OS!=='Windows'||
     process.env.RUNNER_ENVIRONMENT!=='github-hosted'||
     process.env.GITHUB_EVENT_NAME!=='pull_request'||
     !hex(process.env.EXPECTED_HEAD_SHA,40)){
    hold('only exact-head, hosted Windows PR is allowed');
  }
  const workspace=fs.realpathSync(process.env.GITHUB_WORKSPACE||'');
  const temp=fs.realpathSync(process.env.RUNNER_TEMP||'');
  const head=cp.execFileSync('git',['rev-parse','HEAD'],{
    cwd:workspace,encoding:'utf8',stdio:['ignore','pipe','ignore']
  }).trim();
  if(head!==process.env.EXPECTED_HEAD_SHA)hold('checkout does not match reviewed commit');
  const stage=fs.realpathSync(path.resolve(process.argv[2]));
  const out=path.resolve(process.argv[3]);
  if(!under(stage,temp)||!under(path.dirname(out),temp)||fs.existsSync(out))
    hold('unsigned custody stage/output escaped runner temp or output exists');
  const pinned=JSON.parse(fs.readFileSync(
    path.join(workspace,'docs/release/video18b04-five-exe-allowlist.json'),'utf8'));
  const lock=fs.readFileSync(path.join(workspace,'labs/video-lite/Cargo.lock'));
  const {manifest,evidence}=buildUnsignedManifest(stage,pinned,lock,head);
  fs.mkdirSync(out,{mode:0o700});
  fs.writeFileSync(path.join(out,'manifest.json'),manifest,{flag:'wx',mode:0o600});
  fs.writeFileSync(path.join(out,'custody-review.json'),evidence,{flag:'wx',mode:0o600});
  if(fs.readdirSync(out).length!==2)hold('unexpected custody output');
  const independent=buildUnsignedManifest(stage,pinned,lock,head);
  if(!manifest.equals(independent.manifest)||!evidence.equals(independent.evidence))
    hold('unsigned package manifest is not reproducible');
  process.stdout.write('VIDEO_TOOLS_01I_UNSIGNED_CUSTODY_PASS real_amd64_pe=3 upstream_sha256=2 source_lock=1 deterministic=1 signer=none release=0\n');
}
module.exports={names,assertAmd64PE,buildUnsignedManifest,inspectPinnedAllowlist};
if(require.main===module)cli();
