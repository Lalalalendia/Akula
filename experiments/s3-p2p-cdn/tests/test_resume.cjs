// Simulated durable OPFS across browser restarts, digest verification and tamper rollback.
const assert=require('node:assert/strict');
const {createHash}=require('node:crypto');
const {openResumableOutput,withResumeLock}=require('../web/resumable_output.js');
const hash=b=>createHash('sha256').update(b).digest('hex');
const data=Buffer.alloc(16*1024);for(let i=0;i<data.length;i++)data[i]=i%251;
const manifest={version:1,content_id:hash(data),size:data.length,chunks:[0,4096,8192,12288].map(offset=>({offset,size:4096,sha256:hash(data.subarray(offset,offset+4096))}))};
const files=new Map();let stored=0;
const folder={
 async getFileHandle(name,{create}={}) {
 if(!files.has(name)&&create)files.set(name,Buffer.alloc(0));
 const file=()=>({size:files.get(name).length,slice(a,b){return{async arrayBuffer(){return Uint8Array.from(files.get(name).subarray(a,b)).buffer}};}});
 return {getFile:async()=>file(),async createWritable({keepExistingData}={}){
 let bytes=keepExistingData?Buffer.from(files.get(name)):Buffer.alloc(0),pos=0;
 return {async seek(n){pos=n},async truncate(n){bytes=bytes.subarray(0,n);pos=Math.min(pos,n)},
 async write(b){const to=pos+b.length;if(to>bytes.length){const out=Buffer.alloc(to);bytes.copy(out);bytes=out;}Buffer.from(b).copy(bytes,pos);pos=to;},
 async close(){files.set(name,Buffer.from(bytes));stored++;},async abort(){}};
 }};},
 async removeEntry(name){files.delete(name)}
};
const original=Object.getOwnPropertyDescriptor(globalThis,'navigator');
const nav={storage:{async getDirectory(){return{async getDirectoryHandle(){return folder}}},async estimate(){return{quota:100000,usage:0}}},locks:{locked:false,async request(n,o,cb){if(this.locked)return cb(null);this.locked=true;try{return await cb({name:n})}finally{this.locked=false}}}};
Object.defineProperty(globalThis,'navigator',{configurable:true,value:nav});
(async()=>{
 const sha=async b=>hash(b),makeHash=()=>{const h=createHash('sha256');return{update:b=>h.update(b),digestHex:()=>h.digest('hex')}};
 const first=makeHash();
 await withResumeLock(manifest.content_id,async()=>{
 const output=await openResumableOutput(manifest,{hashChunk:sha,hasher:first,checkpointBytes:8192});
 await output.write(data.subarray(0,4096));first.update(data.subarray(0,4096));
 await output.write(data.subarray(4096,8192));first.update(data.subarray(4096,8192));
 });
 assert.equal(stored,1);
 const second=makeHash();
 await withResumeLock(manifest.content_id,async()=>{
 const output=await openResumableOutput(manifest,{hashChunk:sha,hasher:second,checkpointBytes:8192});
 assert.equal(output.recoveredChunks,2);
 for(let i=2;i<4;i++){const p=data.subarray(i*4096,(i+1)*4096);await output.write(p);second.update(p)}
 assert.equal(second.digestHex(),manifest.content_id);assert.equal((await output.finish()).size,data.length);
 });
 const name='resume-'+manifest.content_id+'.bin';
 assert.equal(hash(files.get(name)),manifest.content_id);
 files.get(name)[4096]^=0xff;
 const third=makeHash();
 await withResumeLock(manifest.content_id,async()=>{
 const output=await openResumableOutput(manifest,{hashChunk:sha,hasher:third,checkpointBytes:8192});
 assert.equal(output.recoveredChunks,1);
 for(let i=1;i<4;i++){const p=data.subarray(i*4096,(i+1)*4096);await output.write(p);third.update(p)}
 assert.equal(third.digestHex(),manifest.content_id);await output.finish();
 });
 assert.equal(hash(files.get(name)),manifest.content_id);
 await withResumeLock(manifest.content_id,async()=>assert.rejects(withResumeLock(manifest.content_id,async()=>{}),/другой вкладке/));
 console.log('PASS durable OPFS recovery, bad prefix rollback and same-file lock');
})().catch(e=>{console.error(e);process.exitCode=1}).finally(()=>{if(original)Object.defineProperty(globalThis,'navigator',original);else delete globalThis.navigator;});
