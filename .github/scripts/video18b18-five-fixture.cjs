'use strict';
// Ephemeral *test-only* five-PE shape fixture for real Minisign verification.
// Not actual Windows executables. Never distribute or upload fixture keys.
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const root=process.argv[2];
if(!root || !path.isAbsolute(root) || !root.startsWith(process.env.RUNNER_TEMP+path.sep)
    || process.env.GITHUB_ACTIONS!=='true' || process.env.RUNNER_ENVIRONMENT!=='github-hosted'
    || process.env.GITHUB_EVENT_NAME!=='pull_request'){
  throw Error('VIDEO18B18_HOLD: fixture only in GitHub hosted PR RUNNER_TEMP');
}
fs.mkdirSync(root,{recursive:false,mode:0o700});
const current=path.join(root,'current');
fs.mkdirSync(current,{mode:0o700});
const names=['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
  'akula-yt-dlp.exe','akula-quickjs.exe'];
const members=names.map((filename,index)=>{
  const data=Buffer.alloc(2048,index+17);
  data[0]=0x4d;data[1]=0x5a;
  Buffer.from(filename,'ascii').copy(data,32);
  fs.writeFileSync(path.join(current,filename),data,{mode:0o600,flag:'wx'});
  return {filename,sha256:crypto.createHash('sha256').update(data).digest('hex'),size:data.length};
});
const manifest={
  schema:1,product:'Akula',platform:'windows-x86_64',purpose:'protected-video-five-exe',
  version:'0.3.21',source_commit:'a'.repeat(40),
  cargo_lock_sha256:'b'.repeat(64),components:members
};
fs.writeFileSync(path.join(root,'five-manifest.json'),JSON.stringify(manifest)+'\n',{mode:0o600,flag:'wx'});
process.stdout.write('VIDEO18B18_EPHEMERAL_FIVE_FIXTURE_PASS files=5 manifest=1 release=0\n');
