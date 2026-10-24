/* Bounded output: OPFS for large files, tiny in-memory fallback only. */
(function (global) {
  'use strict';
  const SMALL_LIMIT = 8 * 1024 * 1024;
  const DIR='akula-s3-stream-lab';
  const PARTIAL_TTL=48*60*60*1000;
  const COMPLETE_TTL=30*24*60*60*1000;
  const PARTIAL=/^partial-(\d{13})-([a-f0-9-]{36})\.bin$/;
  const COMPLETE=/^complete-(\d{13})-([a-f0-9-]{36})\.flag$/;

  async function cleanupStaleOutputs(now=Date.now()) {
    const getDir=global.navigator?.storage?.getDirectory;
    if(typeof getDir!=='function')return {removed:0,skipped:true};
    const root=await getDir.call(global.navigator.storage);
    const dir=await root.getDirectoryHandle(DIR,{create:true});
    if(typeof dir.entries!=='function')return {removed:0,skipped:true};
    const entries=[];
    for await (const [name] of dir.entries()) {
      entries.push(name);
      if(entries.length>=2048)break;
    }
    const markers=new Set(entries.filter(n=>COMPLETE.test(n)));
    const paired=new Set();
    let removed=0;
    for(const name of entries) {
      const m=PARTIAL.exec(name);
      if(!m)continue;
      const stamp=Number(m[1]), completed=`complete-${m[1]}-${m[2]}.flag`;
      paired.add(completed);
      const age=now-stamp;
      if(!Number.isFinite(age)||age<0||age<(markers.has(completed)?COMPLETE_TTL:PARTIAL_TTL))continue;
      try {
        await dir.removeEntry(name);
        removed++;
        if(markers.has(completed))await dir.removeEntry(completed);
      } catch(_){ /* active/in-use file: retry later */ }
    }
    for(const name of markers) {
      if(paired.has(name))continue;
      const m=COMPLETE.exec(name);
      if(!m||now-Number(m[1])<COMPLETE_TTL)continue;
      try{await dir.removeEntry(name);removed++;}catch(_){}
    }
    return {removed,skipped:false};
  }
  async function openOutput(size) {
    if (!Number.isSafeInteger(size) || size < 0) throw new RangeError('Invalid length');
    if (global.navigator?.storage?.getDirectory) {
      let dir, filename, marker, writer;
      try {
        try{await cleanupStaleOutputs();}catch(_){ /* best effort */ }
        const usage = await global.navigator.storage.estimate?.();
        if (usage && Number.isFinite(usage.quota) && Number.isFinite(usage.usage) &&
            usage.quota - usage.usage < size + 16 * 1024 * 1024)
          throw new Error('Недостаточно места в браузерном хранилище');
        const root = await global.navigator.storage.getDirectory();
        dir = await root.getDirectoryHandle(DIR, {create:true});
        const stamp=String(Date.now()), token=global.crypto.randomUUID();
        filename=`partial-${stamp}-${token}.bin`;
        marker=`complete-${stamp}-${token}.flag`;
        const handle = await dir.getFileHandle(filename, {create:true});
        writer = await handle.createWritable({keepExistingData:false});
        let total=0,closed=false;
        return {
          mode:'opfs',
          async write(bytes) {
            if (closed || !(bytes instanceof Uint8Array) || total+bytes.length>size)
              throw Error('Invalid output part');
            await writer.write(bytes); total+=bytes.length;
          },
          async finish() {
            if (closed || total!==size) throw Error('Incomplete output');
            closed=true;await writer.close();
            const flagHandle=await dir.getFileHandle(marker,{create:true});
            const flagWriter=await flagHandle.createWritable({keepExistingData:false});
            await flagWriter.close();
            return handle.getFile();
          },
          async discard() {
            closed=true;
            try {await writer.abort();} catch(_){}
            try {await dir.removeEntry(marker);} catch(_){}
            try {await dir.removeEntry(filename);} catch(_){}
          },
          get bytesWritten() {return total;}
        };
      } catch(e) {
        try {await writer?.abort();} catch(_){}
        try {if(dir&&marker)await dir.removeEntry(marker);} catch(_){}
        try {if(dir&&filename)await dir.removeEntry(filename);} catch(_){}
        if(size>SMALL_LIMIT) throw e;
      }
    }
    if(size>SMALL_LIMIT)
      throw Error('Для больших файлов требуется браузер с поддержкой OPFS');
    const parts=[];let total=0,closed=false;
    return {
      mode:'small-memory',
      async write(bytes) {
        if(closed || !(bytes instanceof Uint8Array) || total+bytes.length>size)
          throw Error('Invalid output part');
        parts.push(bytes.slice());total+=bytes.length;
      },
      async finish() {
        if(closed||total!==size)throw Error('Incomplete output');
        closed=true;return new Blob(parts,{type:'application/octet-stream'});
      },
      async discard(){closed=true;parts.length=0;},
      get bytesWritten(){return total;}
    };
  }
  global.AkulaOpenOutput=openOutput;
  global.AkulaCleanupStaleOutputs=cleanupStaleOutputs;
  if(typeof module!=='undefined'&&module.exports)module.exports={openOutput,cleanupStaleOutputs};
})(typeof globalThis!=='undefined'?globalThis:this);
