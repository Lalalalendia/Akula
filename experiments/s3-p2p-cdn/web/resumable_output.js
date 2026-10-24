/* Resumable OPFS output for an immutable, chunk-hashed public object.
 * Caller holds a Web Lock for ContentID during the entire download.
 */
(function (global) {
  'use strict';
  const DIR = 'akula-s3-stream-lab';
  const MAX_CHUNK = 4 * 1024 * 1024;
  const DEFAULT_CHECKPOINT = 8 * 1024 * 1024;
  const HEX = /^[0-9a-f]{64}$/;

  function validateManifest(m) {
    if (!m || m.version !== 1 || !HEX.test(m.content_id) ||
        !Number.isSafeInteger(m.size) || m.size <= 0 ||
        !Array.isArray(m.chunks) || m.chunks.length < 1 || m.chunks.length > 4096) {
      throw new Error('Invalid immutable manifest');
    }
    let next = 0;
    for (const c of m.chunks) {
      if (!c || !Number.isSafeInteger(c.offset) || c.offset !== next ||
          !Number.isSafeInteger(c.size) || c.size < 1 || c.size > MAX_CHUNK ||
          !HEX.test(c.sha256)) throw new Error('Invalid chunk map');
      next += c.size;
      if (!Number.isSafeInteger(next)) throw new Error('Object size overflow');
    }
    if (next !== m.size) throw new Error('Chunk map does not cover object');
  }

  async function withResumeLock(contentId, work) {
    if (!HEX.test(contentId) || typeof work !== 'function') throw new Error('Invalid lock request');
    const locks = global.navigator?.locks;
    if (typeof locks?.request !== 'function') {
      throw new Error('Для безопасной докачки нужен браузер с Web Locks');
    }
    return locks.request('akula-s3-resume:' + contentId,
      {mode: 'exclusive', ifAvailable: true}, lock => {
        if (!lock) throw new Error('Этот файл уже загружается в другой вкладке');
        return work();
      });
  }

  async function openResumableOutput(manifest, {
    hashChunk, hasher, onRecoveredChunk = () => {},
    checkpointBytes = DEFAULT_CHECKPOINT,
  } = {}) {
    validateManifest(manifest);
    if (typeof hashChunk !== 'function' || typeof hasher?.update !== 'function' ||
        !Number.isSafeInteger(checkpointBytes) || checkpointBytes < 1) {
      throw new Error('Invalid streaming verification options');
    }
    if (!global.navigator?.storage?.getDirectory) {
      throw new Error('OPFS необходим для возобновления загрузки');
    }
    const root = await global.navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle(DIR, {create: true});
    // A deterministic name allows recovery without an unreliable in-memory journal.
    const filename = `resume-${manifest.content_id}.bin`;
    const handle = await dir.getFileHandle(filename, {create: true});
    const saved = await handle.getFile();
    const usage = await global.navigator.storage.estimate?.();
    const remaining = Math.max(0, manifest.size - Math.min(saved.size, manifest.size));
    if (usage && Number.isFinite(usage.quota) && Number.isFinite(usage.usage) &&
        usage.quota - usage.usage < remaining + 16 * 1024 * 1024) {
      throw new Error('Недостаточно места для оставшейся части файла');
    }

    let total = 0;
    let chunkIndex = 0;
    // Ignore and truncate any uncommitted tail or corrupted cached part.
    // Only reusing parts whose SHA-256 matches the trusted manifest.
    if (saved.size <= manifest.size) {
      for (const c of manifest.chunks) {
        if (c.offset !== total || c.offset + c.size > saved.size) break;
        const bytes = new Uint8Array(await saved.slice(c.offset, c.offset + c.size).arrayBuffer());
        if (await hashChunk(bytes) !== c.sha256) break;
        hasher.update(bytes);
        onRecoveredChunk(chunkIndex, bytes);
        total += c.size;
        chunkIndex++;
      }
    }
    const recoveredChunks = chunkIndex;
    const recoveredBytes = total;
    let writer = null;
    let lastCheckpoint = total;
    let closed = false;
    async function ensureWritable() {
      if (writer || total === manifest.size) return;
      writer = await handle.createWritable({keepExistingData: true});
      if (saved.size !== total) await writer.truncate(total);
      await writer.seek(total);
    }
    // No second copy of a completely verified object is required.
    if (total < manifest.size) await ensureWritable();

    return {
      mode: 'opfs-resume', recoveredChunks, recoveredBytes,
      get bytesWritten() { return total; },
      async write(bytes) {
        if (closed || !(bytes instanceof Uint8Array) || chunkIndex >= manifest.chunks.length) {
          throw new Error('Unexpected part or closed file');
        }
        const expected = manifest.chunks[chunkIndex];
        if (expected.offset !== total || bytes.length !== expected.size ||
            await hashChunk(bytes) !== expected.sha256) throw new Error('Invalid part before persistence');
        if (!writer) await ensureWritable();
        await writer.write(bytes);
        total += bytes.length;
        chunkIndex++;
        if (total - lastCheckpoint >= checkpointBytes && total < manifest.size) {
          // close() is the durability boundary for OPFS async writable streams.
          await writer.close();
          writer = null;
          lastCheckpoint = total;
          await ensureWritable();
        }
      },
      async pause() {
        if (closed) return;
        closed = true;
        if (writer) { await writer.close(); writer = null; }
      },
      async finish() {
        if (closed || total !== manifest.size) throw new Error('Incomplete output');
        closed = true;
        if (writer) { await writer.close(); writer = null; }
        const completed = await handle.getFile();
        if (completed.size !== manifest.size) throw new Error('Wrong durable file size');
        return completed;
      },
      async discard() {
        closed = true;
        try { await writer?.abort(); } catch (_) { /* closed already */ }
        writer = null;
        try { await dir.removeEntry(filename); } catch (_) { /* best effort */ }
      },
    };
  }

  global.AkulaOpenResumableOutput = openResumableOutput;
  global.AkulaWithResumeLock = withResumeLock;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = {openResumableOutput, withResumeLock, validateManifest};
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
