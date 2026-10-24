/* Strict bounded HTTP Range reader with transient retry; never trusts unverified bytes. */
(function(global){
'use strict';
class PermanentRangeError extends Error {}
const RETRIES=3;
async function checkedRangeGet({url,offset,size,totalSize,expectedSha256,hashHex,fetcher=global.fetch.bind(global),pause=ms=>new Promise(r=>setTimeout(r,ms)),onBytes=()=>{},timeoutMs=25000}){
if(typeof url!=='string'||!(url.startsWith('https://')||url.startsWith('/origin/'))||
!Number.isSafeInteger(offset)||offset<0||!Number.isSafeInteger(size)||size<=0||size>4*1024*1024||
!Number.isSafeInteger(totalSize)||totalSize<=0||offset+size>totalSize||!Number.isSafeInteger(offset+size)||
!/^[a-f0-9]{64}$/.test(expectedSha256)||typeof hashHex!=='function')throw new PermanentRangeError('Invalid range manifest');
const end=offset+size-1, expectedRange=`bytes ${offset}-${end}/${totalSize}`;
for(let attempt=0;attempt<RETRIES;attempt++){
const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),timeoutMs);
try{
const res=await fetcher(url,{headers:{Range:`bytes=${offset}-${end}`},cache:'no-store',signal:controller.signal});
if([429,500,502,503,504].includes(res.status)){const e=new Error('Transient HTTP '+res.status);e.retryable=true;throw e;}
if(res.status!==206&&!(offset===0&&size===totalSize&&res.status===200))throw new PermanentRangeError('Unexpected HTTP '+res.status);
if(res.status===206&&res.headers.get('Content-Range')!==expectedRange)throw new PermanentRangeError('Invalid Content-Range');
const encoding=res.headers.get('Content-Encoding');
if(encoding&&encoding.toLowerCase()!=='identity')throw new PermanentRangeError('Content-Encoding changed object');
const length=res.headers.get('Content-Length');
if(length!==null&&Number(length)!==size)throw new PermanentRangeError('Content-Length mismatch');
if(!res.body?.getReader)throw new PermanentRangeError('Response is not streaming');
const buf=new Uint8Array(size),reader=res.body.getReader();let n=0;
try{while(true){const {done,value}=await reader.read();if(done)break;onBytes(value.byteLength);if(n+value.byteLength>size)throw new PermanentRangeError('Excess bytes');buf.set(value,n);n+=value.byteLength;}}
finally{try{await reader.cancel();}catch(_){}reader.releaseLock();}
if(n!==size)throw new PermanentRangeError('Truncated range');
if(await hashHex(buf)!==expectedSha256)throw new PermanentRangeError('SHA-256 mismatch');
return buf;
}catch(e){
if(e instanceof PermanentRangeError||!(e.retryable||['AbortError','TimeoutError','TypeError'].includes(e.name))||attempt===RETRIES-1)throw e;
await pause(Math.min(250*2**attempt,1000));
}finally{clearTimeout(timer);}
}
throw new Error('Range retry exhausted');
}
global.AkulaCheckedRangeGet=checkedRangeGet;
if(typeof module!=='undefined'&&module.exports)module.exports={checkedRangeGet,PermanentRangeError};
})(typeof globalThis!=='undefined'?globalThis:this);
