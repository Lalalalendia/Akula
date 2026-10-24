const assert=require('node:assert/strict');
const {webcrypto}=require('node:crypto');
const savedNav=Object.getOwnPropertyDescriptor(globalThis,'navigator');
const savedCrypto=Object.getOwnPropertyDescriptor(globalThis,'crypto');
const now=Date.now(),old=now-49*3600000,ancient=now-31*24*3600000,fresh=now-3600000;
const u1='11111111-1111-4111-8111-111111111111',u2='22222222-2222-4222-8222-222222222222',u3='33333333-3333-4333-8333-333333333333';
const files=new Map();
for(const x of [`partial-${old}-${u1}.bin`,`partial-${ancient}-${u2}.bin`,`complete-${ancient}-${u2}.flag`,`partial-${fresh}-${u3}.bin`,'user-file.txt'])files.set(x,0);
const dir={
 async *entries(){for(const n of files.keys())yield[n,{kind:'file'}];},
 async removeEntry(n){if(!files.has(n))throw Error('missing');files.delete(n);},
 async getFileHandle(n,{create=false}={}){
  if(create&&!files.has(n))files.set(n,0);
  if(!files.has(n))throw Error('missing');
  return{async createWritable(){let bytes=0;return{async write(b){bytes+=b.length;},async close(){files.set(n,bytes);},async abort(){}};},async getFile(){return{size:files.get(n)};}};
 }
};
Object.defineProperty(globalThis,'navigator',{configurable:true,value:{storage:{
 async getDirectory(){return{async getDirectoryHandle(){return dir;}};},
 async estimate(){return{quota:2*1024**3,usage:0};}
}}});
Object.defineProperty(globalThis,'crypto',{configurable:true,value:webcrypto});
const {openOutput,cleanupStaleOutputs}=require('../web/bounded_output.js');
(async()=>{
 const a=await cleanupStaleOutputs(now);
 assert.equal(a.removed,2);
 assert(!files.has(`partial-${old}-${u1}.bin`));
 assert(!files.has(`partial-${ancient}-${u2}.bin`));
 assert(!files.has(`complete-${ancient}-${u2}.flag`));
 assert(files.has(`partial-${fresh}-${u3}.bin`));
 assert(files.has('user-file.txt'));
 const f=await openOutput(1234);await f.write(new Uint8Array(1234));
 assert.equal((await f.finish()).size,1234);
 const done=[...files.keys()].find(x=>x.startsWith('complete-'));assert(done);
 await f.discard();
 assert(!files.has(done));
 const broken=await openOutput(12);await broken.write(new Uint8Array(4));
 await assert.rejects(broken.finish(),/Incomplete/);
 await broken.discard();
 console.log('PASS OPFS age-based incomplete cleanup, completion markers, unrelated files, discard');
})().catch(e=>{console.error(e);process.exitCode=1}).finally(()=>{
 if(savedNav)Object.defineProperty(globalThis,'navigator',savedNav);else delete globalThis.navigator;
 if(savedCrypto)Object.defineProperty(globalThis,'crypto',savedCrypto);else delete globalThis.crypto;
});
