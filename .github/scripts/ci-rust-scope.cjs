'use strict';
const { execFileSync } = require('node:child_process');

// Keep the fast lane deliberately narrow. A change to the root workspace,
// build configuration, native UI, or any other core component still receives
// the entire Rust test suite. Documentation-only PRs don't need Rust at all.
function classify(changedPaths) {
  const relevant=changedPaths.filter(p=>p.startsWith('aquilum-app/')
    ||p==='.github/workflows/app-checks.yml');
  if(relevant.length===0) return 'none';
  const syncOnly=relevant.every(p=>p.startsWith('aquilum-app/core/src/sync/')
    ||p==='aquilum-app/core/tests/sync_contract.rs');
  return syncOnly?'sync':'workspace';
}
function changed(base,head='HEAD',cwd=process.cwd()) {
  if(!base||!head||!/^[-.\w/]+$/.test(base)||!/^[-.\w/]+$/.test(head))
    throw Error('Invalid --base or --head');
  // NUL delimiters preserve arbitrary Git filenames. File deletions, renames
  // and type changes must all be considered for CI scope.
  const raw=execFileSync('git',['diff','--name-only','-z','--find-renames',
    base,head,'--'],{encoding:'utf8',maxBuffer:32*1024*1024,cwd});
  return raw.split('\0').filter(Boolean);
}
function main(argv) {
  const args=Object.fromEntries(argv.map(part=>{
    const match=part.match(/^--(base|head)=([^\s]+)$/);
    if(!match) throw Error('Expected --base=<sha> --head=<sha>');
    return [match[1],match[2]];
  }));
  if(!args.base) throw Error('--base is required');
  const paths=changed(args.base,args.head||'HEAD');
  const mode=classify(paths);
  process.stdout.write('mode='+mode+'\n');
}
if(require.main===module) {
  try {main(process.argv.slice(2));}
  catch(err){console.error(err.message);process.exitCode=1;}
}
module.exports={classify,changed,main};
