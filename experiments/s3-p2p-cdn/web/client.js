// Browser-only prototype: SHA-256 verified chunks, explicit upload consent, no S3 credentials.
const $ = id => document.getElementById(id);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const MAX_UPLOAD_BUFFERED = 160 * 1024;
const WIRE_CHUNK = 16 * 1024;
const PEER_TIMEOUT_MS = 12000;
const MAX_CACHE = 96 * 1024 * 1024;

let manifest;
let socket;
let myId;
let connected = false;
let joinResolve;
const joinReady = new Promise(resolve => { joinResolve = resolve; });
let canShare = false;
let downloading = false;
let cacheBytes = 0;
let saveUrl;
let uploadNextAvailableAt = 0;
const cached = new Map(); // chunk number => validated Uint8Array; insertion order used as LRU
const peers = new Map(); // peerId => connection, channel, availability, outstanding requests
const statistics = { httpBytes: 0, p2pBytes: 0, uploadBytes: 0, fromHttp: 0, fromPeers: 0, integrityFailures: 0, peerFailures: 0 };
let announceTimer;

function writeLog(message) {
  const stamp = new Date().toLocaleTimeString('ru-RU', { hour12: false });
  $('log').textContent += `${stamp} ${message}\n`;
  $('log').scrollTop = $('log').scrollHeight;
  console.log('[Akula P2P]', message);
}
function pretty(bytes) { return `${(bytes / 1024 / 1024).toFixed(2)} МиБ`; }
function drawStats() {
  $('httpBytes').textContent = pretty(statistics.httpBytes);
  $('p2pBytes').textContent = pretty(statistics.p2pBytes);
  $('uploadBytes').textContent = pretty(statistics.uploadBytes);
  $('chunks').textContent = `${cached.size}/${manifest?.chunks.length || 0}`;
  $('peerCount').textContent = [...peers.values()].filter(p => p.channel?.readyState === 'open').length;
}
function hasCache(index) { return cached.has(index); }
function cacheChunk(index, data) {
  if (cached.has(index)) cached.delete(index);
  while (cacheBytes + data.byteLength > MAX_CACHE && cached.size) {
    const [key, old] = cached.entries().next().value;
    cached.delete(key);
    cacheBytes -= old.byteLength;
  }
  if (data.byteLength <= MAX_CACHE) {
    cached.set(index, data);
    cacheBytes += data.byteLength;
  }
  drawStats();
  announceSoon();
}
function announceSoon() {
  if (!connected || !myId || announceTimer !== undefined) return;
  announceTimer = setTimeout(() => {
    announceTimer = undefined;
    sendSocket({ type: 'announce', share: canShare, have: canShare ? [...cached.keys()] : [] });
  }, 80);
}
function sendSocket(obj) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(obj));
}
function sendSignal(to, signal) { sendSocket({ type: 'signal', to, signal }); }

function disposePeer(peerId) {
  const peer = peers.get(peerId);
  if (!peer) return;
  peers.delete(peerId);
  for (const pending of peer.pending.values()) {
    clearTimeout(pending.timer);
    pending.reject(Error('Участник отключился'));
  }
  peer.pending.clear();
  try { peer.channel?.close(); peer.rtc?.close(); } catch (_e) { /* already closed */ }
  drawStats();
}
function ensurePeer(peerId, initiate = false) {
  if (peerId === myId) return;
  let peer = peers.get(peerId);
  if (peer?.rtc && peer.rtc.connectionState !== 'closed' && peer.rtc.connectionState !== 'failed') return peer;
  if (peer) disposePeer(peerId);
  const rtc = new RTCPeerConnection({ iceServers: [{ urls: 'stun:stun.l.google.com:19302' }] });
  peer = { id: peerId, rtc, channel: null, have: new Set(), pending: new Map(), nextRid: 1,
    receivingRid: null, incomingCandidates: [], uploadQueue: Promise.resolve(), initiator: initiate };
  peers.set(peerId, peer);
  rtc.onicecandidate = event => { if (event.candidate) sendSignal(peerId, { type: 'candidate', candidate: event.candidate.toJSON() }); };
  rtc.onconnectionstatechange = () => {
    if (['closed', 'failed', 'disconnected'].includes(rtc.connectionState)) {
      writeLog(`Соединение ${peerId}: ${rtc.connectionState}`);
      if (rtc.connectionState !== 'disconnected') disposePeer(peerId);
    }
  };
  rtc.ondatachannel = event => attachDataChannel(peer, event.channel);
  if (initiate) {
    attachDataChannel(peer, rtc.createDataChannel('verified-chunks', { ordered: true }));
    (async () => {
      try {
        const offer = await rtc.createOffer();
        await rtc.setLocalDescription(offer);
        sendSignal(peerId, { type: 'offer', sdp: rtc.localDescription.sdp });
      } catch (e) { writeLog(`Offer ${peerId}: ${e.message}`); disposePeer(peerId); }
    })();
  }
  return peer;
}
async function signalFrom(peerId, signal) {
  let peer = peers.get(peerId);
  if (signal.type === 'offer') {
    if (peer?.rtc?.signalingState !== 'stable') { disposePeer(peerId); peer = undefined; }
    peer ||= ensurePeer(peerId, false);
    await peer.rtc.setRemoteDescription({ type: 'offer', sdp: signal.sdp });
    for (const candidate of peer.incomingCandidates.splice(0)) await peer.rtc.addIceCandidate(candidate);
    const answer = await peer.rtc.createAnswer();
    await peer.rtc.setLocalDescription(answer);
    sendSignal(peerId, { type: 'answer', sdp: peer.rtc.localDescription.sdp });
  } else if (signal.type === 'answer') {
    if (!peer) return;
    if (peer.rtc.signalingState !== 'have-local-offer') return;
    await peer.rtc.setRemoteDescription({ type: 'answer', sdp: signal.sdp });
    for (const candidate of peer.incomingCandidates.splice(0)) await peer.rtc.addIceCandidate(candidate);
  } else if (signal.type === 'candidate') {
    peer ||= ensurePeer(peerId, false);
    if (peer.rtc.remoteDescription) await peer.rtc.addIceCandidate(signal.candidate);
    else peer.incomingCandidates.push(signal.candidate);
  }
}
function attachDataChannel(peer, channel) {
  peer.channel = channel;
  channel.binaryType = 'arraybuffer';
  channel.onopen = () => { writeLog(`Прямой канал открыт: ${peer.id}`); drawStats(); };
  channel.onclose = () => { writeLog(`Канал закрыт: ${peer.id}`); drawStats(); };
  channel.onerror = () => { writeLog(`Ошибка канала: ${peer.id}`); };
  channel.onmessage = event => {
    try {
      if (typeof event.data === 'string') {
        const cmd = JSON.parse(event.data);
        receiveCommand(peer, cmd);
      } else if (event.data instanceof ArrayBuffer) {
        receiveBytes(peer, new Uint8Array(event.data));
      }
    } catch (e) { writeLog(`Ошибка сообщения ${peer.id}: ${e.message}`); }
  };
}
function sendChannel(peer, cmd) {
  if (peer.channel?.readyState !== 'open') throw Error('P2P канал закрыт');
  peer.channel.send(JSON.stringify(cmd));
}
function receiveCommand(peer, cmd) {
  if (cmd.t === 'get') {
    if (!Number.isSafeInteger(cmd.rid) || !Number.isSafeInteger(cmd.n)) return;
    peer.uploadQueue = peer.uploadQueue.catch(() => {}).then(() => sendPeerChunk(peer, cmd.rid, cmd.n)).catch(e => writeLog(`Отдача ${peer.id}: ${e.message}`));
    return;
  }
  const pending = peer.pending.get(cmd.rid);
  if (!pending) return;
  if (cmd.t === 'missing' || cmd.t === 'fail') {
    pending.reject(Error('У другого участника нет нужной части'));
  } else if (cmd.t === 'start') {
    if (peer.receivingRid !== null || cmd.n !== pending.index || cmd.length !== manifest.chunks[pending.index].size) {
      pending.reject(Error('Неверный заголовок части'));
      return;
    }
    pending.started = true;
    peer.receivingRid = cmd.rid;
  } else if (cmd.t === 'end') {
    if (!pending.started || peer.receivingRid !== cmd.rid || pending.received !== manifest.chunks[pending.index].size) {
      pending.reject(Error('Неполная P2P-передача'));
      return;
    }
    peer.receivingRid = null;
    const output = new Uint8Array(pending.received);
    let offset = 0;
    for (const data of pending.parts) { output.set(data, offset); offset += data.length; }
    pending.resolve(output);
  }
}
function receiveBytes(peer, data) {
  const pending = peer.pending.get(peer.receivingRid);
  if (!pending?.started || pending.received + data.byteLength > manifest.chunks[pending.index].size) {
    return;
  }
  pending.parts.push(data);
  pending.received += data.byteLength;
}
async function awaitBufferSpace(channel) {
  while (channel.readyState === 'open' && channel.bufferedAmount > MAX_UPLOAD_BUFFERED) {
    await new Promise((resolve, reject) => {
      let finished = false;
      const cleanup = () => { channel.removeEventListener('bufferedamountlow', onLow); channel.removeEventListener('close', onClose); clearTimeout(timer); };
      const onLow = () => { if (finished) return; finished = true; cleanup(); resolve(); };
      const onClose = () => { if (finished) return; finished = true; cleanup(); reject(Error('Канал закрыт')); };
      channel.bufferedAmountLowThreshold = 64 * 1024;
      channel.addEventListener('bufferedamountlow', onLow);
      channel.addEventListener('close', onClose);
      const timer = setTimeout(onLow, 250);
    });
  }
  if (channel.readyState !== 'open') throw Error('Канал закрыт');
}
async function throttleUpload(bytes) {
  // Global limiter shared between all peers on this tab, not per-connection.
  const mibPerSecond = Math.max(1, Math.min(16, Number($('uploadLimit').value) || 8));
  const permittedAt = Math.max(performance.now(), uploadNextAvailableAt);
  uploadNextAvailableAt = permittedAt + bytes / (mibPerSecond * 1024 * 1024) * 1000;
  const delay = permittedAt - performance.now();
  if (delay > 1) await sleep(delay);
}
async function sendPeerChunk(peer, rid, index) {
  const channel = peer.channel;
  const data = cached.get(index);
  if (!canShare || !data || !channel || channel.readyState !== 'open' || index < 0 || index >= manifest.chunks.length) {
    if (channel?.readyState === 'open') sendChannel(peer, { t: 'missing', rid });
    return;
  }
  sendChannel(peer, { t: 'start', rid, n: index, length: data.byteLength });
  try {
    for (let offset = 0; offset < data.length; offset += WIRE_CHUNK) {
      if (!canShare) throw Error('Отдача отключена');
      await awaitBufferSpace(channel);
      const frame = data.subarray(offset, offset + WIRE_CHUNK);
      await throttleUpload(frame.byteLength);
      if (!canShare) throw Error('Отдача отключена');
      channel.send(frame);
      statistics.uploadBytes += frame.byteLength;
    }
    sendChannel(peer, { t: 'end', rid });
    drawStats();
  } catch (error) {
    if (channel.readyState === 'open') sendChannel(peer, { t: 'fail', rid });
    throw error;
  }
}
function getChunkFromPeer(peer, index) {
  return new Promise((resolve, reject) => {
    if (peer.channel?.readyState !== 'open') return reject(Error('Канал ещё не открыт'));
    const rid = peer.nextRid++;
    const timer = setTimeout(() => { pending.reject(Error('P2P истекло время ожидания')); }, PEER_TIMEOUT_MS);
    const pending = { index, rid, received: 0, parts: [], started: false, timer,
      resolve: value => { clearTimeout(timer); peer.pending.delete(rid); if (peer.receivingRid === rid) peer.receivingRid = null; resolve(value); },
      reject: error => { clearTimeout(timer); peer.pending.delete(rid); if (peer.receivingRid === rid) peer.receivingRid = null; reject(error); },
    };
    peer.pending.set(rid, pending);
    try { sendChannel(peer, { t: 'get', rid, n: index }); }
    catch (error) { pending.reject(error); }
  });
}
async function verifyChunk(index, data) {
  const expected = manifest.chunks[index];
  if (data.byteLength !== expected.size) return false;
  const hash = await crypto.subtle.digest('SHA-256', data);
  const hex = Array.from(new Uint8Array(hash), v => v.toString(16).padStart(2, '0')).join('');
  return hex === expected.sha256;
}
function eligiblePeers(index) {
  return [...peers.values()]
    .filter(p => p.channel?.readyState === 'open' && p.have.has(index))
    .sort((a, b) => a.pending.size - b.pending.size);
}
async function fetchChunk(index) {
  const expected = manifest.chunks[index];
  const candidates = eligiblePeers(index);
  for (const peer of candidates) {
    try {
      const bytes = await getChunkFromPeer(peer, index);
      if (!await verifyChunk(index, bytes)) {
        statistics.integrityFailures++;
        throw Error('Неверная контрольная сумма P2P');
      }
      statistics.p2pBytes += bytes.byteLength;
      statistics.fromPeers++;
      cacheChunk(index, bytes);
      drawStats();
      return bytes;
    } catch (e) { statistics.peerFailures++; writeLog(`P2P часть ${index}: ${e.message}, пробую другой источник`); }
  }
  const hexHash = async bytes => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)),
    v => v.toString(16).padStart(2, '0')).join('');
  const arr = await window.AkulaCheckedRangeGet({
    url:manifest.origin_url, offset:expected.offset, size:expected.size,
    totalSize:manifest.size, expectedSha256:expected.sha256,
    hashHex:hexHash, onBytes:n=>{statistics.httpBytes+=n;}
  });
  statistics.fromHttp++;
  cacheChunk(index, arr);
  drawStats();
  return arr;
}
async function waitForPeer(maxMs = 3500) {
  const start = performance.now();
  while (performance.now() - start < maxMs) {
    if ([...peers.values()].some(p => p.have.size > 0 && p.channel?.readyState === 'open')) return;
    await sleep(40);
  }
}
async function retrieve() {
  if (!manifest || downloading) return;
  downloading = true;
  $('download').disabled = true;
  $('save').hidden = true;
  $('status').textContent = 'Получаю части...';
  $('progress').value = 0;
  const started = performance.now();
  const baseline = { ...statistics };
  try {
    await joinReady;
    if ([...peers.values()].some(p => p.have.size > 0)) await waitForPeer();
    const supportsResume = typeof navigator !== 'undefined' &&
      !!(navigator.storage?.getDirectory && navigator.locks?.request);
    const fullHash = new window.IncrementalSha256();
    const verifyHex = async bytes => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),
      v=>v.toString(16).padStart(2,'0')).join('');
    return await (supportsResume ? window.AkulaWithResumeLock(manifest.content_id,runDownload) : runDownload());
    async function runDownload() {
    const output = supportsResume
      ? await window.AkulaOpenResumableOutput(manifest,{
          hashChunk:verifyHex,hasher:fullHash,
          onRecoveredChunk:()=>{},
        })
      : await window.AkulaOpenOutput(manifest.size);
    try {
      let next = supportsResume ? output.recoveredChunks : 0;
      const windowSize = Math.min(4, Math.max(1, manifest.chunks.length));
      const inflight = new Map();
      const getPart = n => {
        if (!inflight.has(n)) {
          const promise = hasCache(n) ? Promise.resolve(cached.get(n)) : fetchChunk(n);
          inflight.set(n, promise);
        }
        return inflight.get(n);
      };
      for (let j = 0; j < windowSize; j++) if (next+j < manifest.chunks.length) getPart(next+j);
      for (let n = next; n < manifest.chunks.length; n++) {
        const bytes = await getPart(n);
        inflight.delete(n);
        if (!await verifyChunk(n, bytes)) throw Error('Ошибка проверки части перед записью');
        await output.write(bytes);
        fullHash.update(bytes);
        next++;
        const following = n + windowSize;
        if (following < manifest.chunks.length) getPart(following);
        $('progress').value = 100 * next / manifest.chunks.length;
        $('status').textContent = `Проверено ${next}/${manifest.chunks.length} частей`;
      }
      if (fullHash.digestHex() !== manifest.content_id) throw Error('Не совпадает SHA-256 всего файла');
      const blob = await output.finish();
      if (saveUrl) URL.revokeObjectURL(saveUrl);
      saveUrl = URL.createObjectURL(blob);
      $('save').href = saveUrl;
      $('save').download = manifest.name;
      $('save').hidden = false;
    } catch (error) {
      if (supportsResume) await output.pause();
      else await output.discard();
      throw error;
    }
    $('progress').value = 100;
    const seconds = ((performance.now() - started) / 1000).toFixed(2);
    const http = statistics.httpBytes - baseline.httpBytes;
    const p2p = statistics.p2pBytes - baseline.p2pBytes;
    $('status').textContent = `Готово, SHA-256 совпадает. HTTP ${pretty(http)}, P2P ${pretty(p2p)}, ${seconds} с`;
    writeLog($('status').textContent);
    announceSoon();
    }
  } catch (e) {
    $('status').textContent = `Ошибка: ${e.message}`;
    writeLog($('status').textContent);
  } finally {
    downloading = false;
    $('download').disabled = false;
    drawStats();
  }
}
async function onWsMessage(raw) {
  const msg = JSON.parse(raw);
  if (msg.type === 'welcome') {
    myId = msg.id;
    sendSocket({ type: 'join', content_id: manifest.content_id });
  } else if (msg.type === 'joined') {
    connected = true;
    for (const other of msg.peers) {
      const peer = ensurePeer(other.id, true);
      peer.have = new Set(other.have);
    }
    joinResolve();
    announceSoon();
    writeLog(`Подключено к указателю источников. Найдено раздающих: ${msg.peers.length}`);
  } else if (msg.type === 'availability') {
    if (msg.id === myId) return;
    if (msg.have.length === 0) { const p = peers.get(msg.id); if (p) p.have.clear(); return; }
    const peer = ensurePeer(msg.id, true);
    peer.have = new Set(msg.have);
  } else if (msg.type === 'signal') {
    try { await signalFrom(msg.from, msg.signal); }
    catch (e) { writeLog(`Согласование ${msg.from}: ${e.message}`); }
  } else if (msg.type === 'left') {
    disposePeer(msg.id);
  } else if (msg.type === 'error') {
    writeLog(`Сервер обнаружения: ${msg.message}`);
  }
}
async function init() {
  if (!window.RTCPeerConnection || !window.crypto?.subtle) throw Error('Нужен браузер с WebRTC и Web Crypto (localhost или HTTPS)');
  const response = await fetch('/manifest.json', { cache: 'no-store' });
  if (!response.ok) throw Error('Не удалось получить манифест');
  manifest = await response.json();
  if (manifest.version !== 1 || !/^[0-9a-f]{64}$/.test(manifest.content_id) || !Array.isArray(manifest.chunks)) {
    throw Error('Неверный манифест');
  }
  $('name').textContent = manifest.name;
  $('filemeta').textContent = `${pretty(manifest.size)}, ${manifest.chunks.length} частей по ${pretty(manifest.chunk_size)}; SHA-256: ${manifest.content_id.slice(0, 24)}…`;
  drawStats();
  $('share').addEventListener('change', event => {
    canShare = event.target.checked;
    announceSoon();
    writeLog(`Добровольная раздача: ${canShare ? 'включена' : 'выключена'}`);
  });
  $('download').addEventListener('click', retrieve);
  socket = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`);
  socket.onmessage = ev => { onWsMessage(ev.data).catch(e => writeLog(e.message)); };
  socket.onerror = () => writeLog('Ошибка сервера обнаружения');
  socket.onclose = () => { connected = false; for (const id of [...peers.keys()]) disposePeer(id); writeLog('Сервер обнаружения недоступен, HTTP-загрузка остаётся доступной'); joinResolve(); };
  $('download').disabled = false;
  $('status').textContent = 'Готово. Для P2P откройте вторую вкладку после загрузки материала в первую.';
}
window.__akula = { get metrics() { return { ...statistics, cachedCount: cached.size, peerCount: [...peers.values()].filter(p => p.channel?.readyState === 'open').length }; }, get manifest() { return manifest; }, get cache() { return cached; }, get peers() { return peers; }, start: retrieve };
init().catch(e => { $('status').textContent = `Ошибка: ${e.message}`; writeLog(e.message); });