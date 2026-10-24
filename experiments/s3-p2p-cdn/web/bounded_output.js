/* Bounded output: OPFS for large files, tiny in-memory fallback only. */
(function (global) {
  'use strict';
  const SMALL_LIMIT = 8 * 1024 * 1024;
  async function openOutput(size) {
    if (!Number.isSafeInteger(size) || size < 0) throw new RangeError('Invalid length');
    if (global.navigator?.storage?.getDirectory) {
      let dir, filename, writer;
      try {
        const usage = await global.navigator.storage.estimate?.();
        if (usage && Number.isFinite(usage.quota) && Number.isFinite(usage.usage) &&
            usage.quota - usage.usage < size + 16 * 1024 * 1024)
          throw new Error('Недостаточно места в браузерном хранилище');
        const root = await global.navigator.storage.getDirectory();
        dir = await root.getDirectoryHandle('akula-s3-stream-lab', {create:true});
        filename = 'partial-' + global.crypto.randomUUID() + '.bin';
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
            closed=true;await writer.close();return handle.getFile();
          },
          async discard() {
            closed=true;
            try {await writer.abort();} catch(_){}
            try {await dir.removeEntry(filename);} catch(_){}
          },
          get bytesWritten() {return total;}
        };
      } catch(e) {
        try {await writer?.abort();} catch(_){}
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
  if(typeof module!=='undefined'&&module.exports)module.exports={openOutput};
})(typeof globalThis!=='undefined'?globalThis:this);
