// Real Chromium + persistent OPFS; no fake storage, WebRTC or HTTP implementation.
// Requires a localhost lab server with a known immutable test fixture.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {chromium} = require('playwright');
const BASE = process.env.AKULA_LAB_URL || 'http://127.0.0.1:8766';
const SIZE = Number(process.env.AKULA_LAB_SIZE || 64 * 1024 * 1024);
const MODE = process.env.AKULA_LAB_MODE || 'resume'; // resume or full
const OUTPUT = path.resolve(__dirname, '..', 'test-results');
fs.mkdirSync(OUTPUT, {recursive:true});
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'akula-s3-profile-'));
const evidence = {mode:MODE, size:SIZE, browser:'chromium-persistent', origin:BASE, observed:[], passed:false,
  limitation:'Localhost and same runner; not WAN, cloud S3, or an operating-system power loss.'};
const delay=ms=>new Promise(r=>setTimeout(r,ms));

async function stats() {
  const response=await fetch(BASE+'/stats');
  assert.equal(response.status,200);
  return response.json();
}
function rssMiB() {
  try {
    const rows=execFileSync('ps',['-eo','pid=,ppid=,rss=,args='],{encoding:'utf8',maxBuffer:3*1024*1024})
      .split('\n').map(x=>x.trim()).filter(Boolean).map(x=>{
        const m=/^(\d+)\s+(\d+)\s+(\d+)\s+(.*)$/.exec(x);
        return m?{pid:+m[1],ppid:+m[2],rss:+m[3],args:m[4]}:null;
      }).filter(Boolean);
    const root=rows.find(x=>x.args.includes('--user-data-dir='+profile) && !x.args.includes('--type='));
    if(!root) return null;
    const pids=new Set([root.pid]);
    for(let i=0;i<6;i++) for(const x of rows) if(pids.has(x.ppid))pids.add(x.pid);
    return Number((rows.filter(x=>pids.has(x.pid)).reduce((acc,x)=>acc+x.rss,0)/1024).toFixed(1));
  }catch(_){return null;}
}
async function sample(page,label) {
  let heap=null;
  try{heap=await page.evaluate(()=>performance.memory?.usedJSHeapSize??null);}catch(_){}
  const reading={phase:label,elapsedMs:Date.now()-start,rssMiB:rssMiB(),
    jsHeapMiB:heap==null?null:Number((heap/1024/1024).toFixed(1))};
  evidence.observed.push(reading);
  return reading;
}
const start=Date.now();
async function open() {
  const context=await chromium.launchPersistentContext(profile,{
    headless:true,args:['--no-sandbox','--disable-dev-shm-usage','--enable-precise-memory-info'],
    viewport:{width:1000,height:800},
  });
  const page=context.pages()[0]||await context.newPage();
  page.on('pageerror',e=>{(evidence.pageErrors??=[]).push(String(e.message).slice(0,500));});
  await page.goto(BASE,{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>window.__akula?.manifest && document.querySelector('#download')?.disabled===false,
    undefined,{timeout:30000});
  assert.equal((await page.evaluate(()=>window.__akula.manifest.size)),SIZE);
  return {context,page};
}
async function done(page,timeout=240000) {
  await page.waitForFunction(()=>/^(Готово, SHA-256 совпадает|Ошибка:)/.test(
    document.querySelector('#status')?.textContent||''),undefined,{timeout});
  const status=await page.locator('#status').textContent();
  assert.match(status,/^Готово, SHA-256 совпадает/,status);
  return status;
}
async function persisted(page) {
  return page.evaluate(async()=>{
    const cid=window.__akula.manifest.content_id;
    const root=await navigator.storage.getDirectory();
    const dir=await root.getDirectoryHandle('akula-s3-stream-lab');
    const handle=await dir.getFileHandle('resume-'+cid+'.bin');
    return {size:(await handle.getFile()).size,contentId:cid};
  });
}
async function verifyFile(page) {
  return page.evaluate(async()=>{
    const cid=window.__akula.manifest.content_id;
    const root=await navigator.storage.getDirectory();
    const dir=await root.getDirectoryHandle('akula-s3-stream-lab');
    const handle=await dir.getFileHandle('resume-'+cid+'.bin');
    const file=await handle.getFile(),hash=new IncrementalSha256();
    for(let off=0;off<file.size;off+=1024*1024) {
      const part=new Uint8Array(await file.slice(off,off+1024*1024).arrayBuffer());
      hash.update(part);
    }
    return {size:file.size,sha256:hash.digestHex(),expected:cid};
  });
}
let ctx;
try{
  const before=await stats();
  let item=await open();ctx=item.context;let page=item.page;
  await sample(page,'start');
  if(MODE==='resume'){
    // Throttle real HTTP ranges, not OPFS, to close mid-transfer after a durable checkpoint.
    await page.route('**/origin/demo.bin',async route=>{
      await delay(55);
      try {await route.continue();}catch(_){} 
    });
    await page.locator('#download').click();
    await page.waitForFunction(()=>Number(document.querySelector('#progress')?.value)>=25,
      undefined,{timeout:120000});
    const mid=await persisted(page);
    assert(mid.size>=8*1024*1024 && mid.size<SIZE,
      'Must interrupt after at least one durable OPFS checkpoint, before completion');
    evidence.interruptedAt=mid;
    await sample(page,'interrupted');
    await ctx.close();ctx=null;
    const atRestart=await stats();
    item=await open();ctx=item.context;page=item.page;
    const saved=await persisted(page);
    evidence.savedAtRestart=saved;
    assert(saved.size>=8*1024*1024 && saved.size<SIZE);
    const rangeStarts=[];
    await page.route('**/origin/demo.bin',async route=>{
      const range=route.request().headers().range||'';
      const m=/^bytes=(\d+)-(\d+)$/.exec(range);
      if(m)rangeStarts.push(Number(m[1]));
      await route.continue();
    });
    await page.locator('#download').click();
    evidence.finalStatus=await done(page);
    await sample(page,'resumed-complete');
    const after=await stats();
    evidence.resumedRangeMin=rangeStarts.length?Math.min(...rangeStarts):null;
    evidence.resumedOriginRequests=after.origin_requests-atRestart.origin_requests;
    evidence.firstOriginRequests=atRestart.origin_requests-before.origin_requests;
    assert(rangeStarts.length>0,'Resume must request missing ranges');
    assert(evidence.resumedRangeMin>=saved.size,
      'Recovered verified prefix must not be downloaded again');
    assert(evidence.resumedOriginRequests<=Math.ceil((SIZE-saved.size)/262144)+8,
      'Too many origin GET after resume');
  }else if(MODE==='full'){
    await page.locator('#download').click();
    let finished=false;
    for(let i=0;i<600;i++){
      const status=await page.locator('#status').textContent();
      if(status?.startsWith('Готово, SHA-256 совпадает')){finished=true;break;}
      if(status?.startsWith('Ошибка:'))throw Error(status);
      if(i%5===0)await sample(page,'in-progress');
      await delay(1000);
    }
    assert(finished,'Full streaming did not finish within deadline');
    evidence.finalStatus=await done(page);
    await sample(page,'full-complete');
  }else throw Error('Invalid mode');
  evidence.finalFile=await verifyFile(page);
  assert.equal(evidence.finalFile.size,SIZE);
  assert.equal(evidence.finalFile.sha256,evidence.finalFile.expected);
  evidence.passed=true;
  console.log(JSON.stringify(evidence,null,2));
}catch(e){
  evidence.error=String(e.stack||e);
  console.error(evidence.error);
  throw e;
}finally{
  fs.writeFileSync(path.join(OUTPUT,'browser-'+MODE+'-result.json'),
    JSON.stringify(evidence,null,2)+'\n');
  try{await ctx?.close();}catch(_){}
  fs.rmSync(profile,{recursive:true,force:true,maxRetries:4,retryDelay:250});
}
