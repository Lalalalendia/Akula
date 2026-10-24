const {IncrementalSha256} = require('../web/sha256_stream.js');
const {openOutput} = require('../web/bounded_output.js');
const {createHash,randomBytes,webcrypto} = require('node:crypto');
const assert = require('node:assert/strict');
(async () => {
  for(const length of [0,1,3,55,56,63,64,65,127,128,129,1024,262144,1048576]){
    const bytes=randomBytes(length);
    for(const stride of [1,7,64,65536]){
      const h=new IncrementalSha256();
      for(let i=0;i<length;i+=stride)h.update(bytes.subarray(i,Math.min(length,i+stride)));
      assert.equal(h.digestHex(),createHash('sha256').update(bytes).digest('hex'));
    }
  }
  const data=Buffer.alloc(256*1024,73),h=new IncrementalSha256(),control=createHash('sha256');
  for(let i=0;i<256;i++){h.update(data);control.update(data);}
  assert.equal(h.digestHex(),control.digest('hex'),'64 MiB hash must match');
  const small=await openOutput(3);
  await small.write(new Uint8Array([1,2,3]));
  assert.equal((await small.finish()).size,3);
  await assert.rejects(openOutput(1024*1024*1024),/OPFS/,'no full-file fallback');
  const original=Object.getOwnPropertyDescriptor(globalThis,'navigator');
  const originalCrypto=Object.getOwnPropertyDescriptor(globalThis,'crypto');
  let bytes=0,writes=0,deleted=false;
  const writer={async write(x){bytes+=x.length;writes++;},async close(){},async abort(){}};
  const handle={async createWritable(){return writer;},async getFile(){return {size:bytes};}};
  const folder={async getFileHandle(){return handle;},async removeEntry(){deleted=true;}};
  Object.defineProperty(globalThis,'navigator',{configurable:true,value:{storage:{
    async estimate(){return {quota:2*1024**3,usage:0};},
    async getDirectory(){return {async getDirectoryHandle(){return folder;}};}
  }}});
  Object.defineProperty(globalThis,'crypto',{configurable:true,value:webcrypto});
  try{
    const sink=await openOutput(1024*1024*1024);
    assert.equal(sink.mode,'opfs');
    const block=new Uint8Array(1024*1024);
    for(let i=0;i<1024;i++)await sink.write(block);
    assert.equal((await sink.finish()).size,1024*1024*1024);
    assert.equal(writes,1024);
    await sink.discard();assert.equal(deleted,true);
  }finally{
    if(original)Object.defineProperty(globalThis,'navigator',original);
    else delete globalThis.navigator;
    if(originalCrypto)Object.defineProperty(globalThis,'crypto',originalCrypto);
    else delete globalThis.crypto;
  }
  console.log('PASS bounded output: SHA-256, 64MiB incremental and simulated 1GiB OPFS');
})().catch(e=>{console.error(e);process.exitCode=1;});
