const {checkedRangeGet,PermanentRangeError}=require('../web/range_reader.js');
const {createHash}=require('node:crypto'),assert=require('node:assert/strict');
const source=Buffer.alloc(262144);for(let i=0;i<source.length;i++)source[i]=i%251;
const offset=13,size=512,hash=b=>createHash('sha256').update(b).digest('hex'),sha=hash(source.subarray(offset,offset+size));
const opts={url:'/origin/file',offset,size,totalSize:source.length,expectedSha256:sha,hashHex:async b=>hash(b),pause:async()=>{}};
const range=`bytes ${offset}-${offset+size-1}/${source.length}`;
const good=()=>new Response(source.subarray(offset,offset+size),{status:206,headers:{'Content-Range':range}});
(async()=>{
let n=0,bytes=0;
assert.equal((await checkedRangeGet({...opts,fetcher:async()=>{if(++n<3)throw new TypeError('lost');return good()},onBytes:x=>bytes+=x})).length,size);
assert.equal(n,3);assert.equal(bytes,size);
n=0;await checkedRangeGet({...opts,fetcher:async()=>{if(++n<3)return new Response('',{status:503});return good()}});assert.equal(n,3);
const bad=[new Response(source.subarray(offset,offset+size),{status:200}),
new Response(source.subarray(offset,offset+size),{status:206,headers:{'Content-Range':'bytes 0-511/262144'}}),
new Response(source.subarray(offset,offset+size-1),{status:206,headers:{'Content-Range':range}}),
new Response(Buffer.alloc(size),{status:206,headers:{'Content-Range':range}}),
new Response('',{status:412}),new Response('',{status:416}),
new Response(source.subarray(offset,offset+size),{status:206,headers:{'Content-Range':range,'Content-Encoding':'gzip'}})];
for(const res of bad){n=0;await assert.rejects(checkedRangeGet({...opts,fetcher:async()=>{n++;return res.clone()}}),PermanentRangeError);assert.equal(n,1);}
console.log('PASS Range transient retry and 7 fail-closed cases');
})().catch(e=>{console.error(e);process.exitCode=1});
