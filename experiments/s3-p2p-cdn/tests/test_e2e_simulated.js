// In-process simulated WebRTC endpoints; real WS + real HTTP Range via aiohttp.
// This is NOT a physical or Chromium WebRTC test; it checks client protocol logic.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const { webcrypto, randomUUID } = require('node:crypto');
const { performance } = require('node:perf_hooks');
const script = fs.readFileSync(path.resolve(__dirname, '../web/client.js'), 'utf8');
const hashScript = fs.readFileSync(path.resolve(__dirname, '../web/sha256_stream.js'), 'utf8');
const outputScript = fs.readFileSync(path.resolve(__dirname, '../web/bounded_output.js'), 'utf8');
const rangeScript = fs.readFileSync(path.resolve(__dirname, '../web/range_reader.js'), 'utf8');

class Emitter {
  listeners = new Map();
  addEventListener(name, fn) { if (!this.listeners.has(name)) this.listeners.set(name,new Set()); this.listeners.get(name).add(fn); }
  removeEventListener(name, fn) { this.listeners.get(name)?.delete(fn); }
  emit(name, arg) { for (const f of this.listeners.get(name) || []) f(arg); }
}
class FakeChannel extends Emitter {
  readyState='connecting'; bufferedAmount=0; bufferedAmountLowThreshold=65536; onmessage; onopen; onclose; onerror;
  send(data) {
    if (this.readyState !== 'open') throw Error('channel not open');
    const wire = typeof data === 'string' ? data : data.buffer.slice(data.byteOffset, data.byteOffset+data.byteLength);
    const length = typeof data === 'string' ? data.length : wire.byteLength;
    this.bufferedAmount += length;
    setImmediate(() => {
      this.bufferedAmount -= length;
      if (this.bufferedAmount <= this.bufferedAmountLowThreshold) this.emit('bufferedamountlow');
      if (this.twin.readyState === 'open') this.twin.onmessage?.({data:wire});
    });
  }
  open() { this.readyState='open'; this.onopen?.(); this.emit('open'); }
  close() { if(this.readyState==='closed')return;this.readyState='closed';this.onclose?.();this.emit('close'); if(this.twin?.readyState !== 'closed')this.twin.close(); }
}
const offers = new Map();
class FakePC {
  localDescription; remoteDescription; onicecandidate; onconnectionstatechange; ondatachannel; signalingState='stable'; connectionState='new';
  constructor() { this.session=randomUUID(); }
  createDataChannel() { this.channel = new FakeChannel(); return this.channel; }
  async createOffer() { return {type:'offer',sdp:this.session}; }
  async setLocalDescription(desc) {
    this.localDescription=desc;
    if (desc.type==='offer') {this.signalingState='have-local-offer';offers.set(desc.sdp,this);}
    else {this.signalingState='stable';}
  }
  async setRemoteDescription(desc) {
    this.remoteDescription=desc;
    if(desc.type==='offer') {
      this.signalingState='have-remote-offer';
      const sender=offers.get(desc.sdp);if(!sender)throw Error('offer not found');
      this.counterpart=sender;
      sender.counterpart=this;
      const chan=new FakeChannel();
      chan.twin=sender.channel;
      sender.channel.twin=chan;
      this.channel=chan;
      this.ondatachannel?.({channel:chan});
    } else if (desc.type==='answer') {
      this.signalingState='stable';
      const other=this.counterpart;
      if(!other)throw Error('answer not paired');
      setImmediate(()=>{this.connectionState='connected';other.connectionState='connected';this.channel.open();other.channel.open();this.onconnectionstatechange?.();other.onconnectionstatechange?.();});
    }
  }
  async createAnswer(){return {type:'answer',sdp:'answer-'+this.remoteDescription.sdp};}
  async addIceCandidate(){}
  close(){if(this.connectionState==='closed')return;this.connectionState='closed';this.channel?.close();this.onconnectionstatechange?.();}
}
class FakeElement extends Emitter {
  textContent=''; disabled=false; value=0; hidden=false; checked=false; scrollTop=0; scrollHeight=0;
  click(){this.emit('click',{target:this});}
  toggleShare(v){this.checked=v;this.emit('change',{target:this});}
}
function tab(label) {
  const elements = new Map();
  const doc={getElementById(id){if(!elements.has(id))elements.set(id,new FakeElement());return elements.get(id);}};
  const output=[];
  const context=vm.createContext({
    document:doc, location:{protocol:'http:',host:'127.0.0.1:8765'},
    WebSocket, RTCPeerConnection:FakePC, crypto:webcrypto,
    Blob, URL, Uint8Array, ArrayBuffer, Set, Map, Promise, Headers, fetch:(u,opts)=>fetch(new URL(u,"http://127.0.0.1:8765/").href,opts), performance, Date,
    setTimeout, clearTimeout, console:{log:(...xs)=>output.push(xs.join(' '))},
    window:{RTCPeerConnection:FakePC,crypto:webcrypto},
  });
  vm.runInContext(hashScript,context,{filename:'sha256_stream.js'});
  vm.runInContext(outputScript,context,{filename:'bounded_output.js'});
  vm.runInContext(rangeScript,context,{filename:'range_reader.js'});
  vm.runInContext(script,context,{filename:'client.js'});
  return {label,elements,context,output, get api(){return context.window.__akula;},get el(){return doc.getElementById.bind(doc);}};
}
async function until(fn, timeout=10000, label='condition') {
  const started=Date.now();while(Date.now()-started<timeout){if(fn())return;await new Promise(r=>setTimeout(r,50));}throw Error('Timed out: '+label);
}
async function download(t) {
  await until(()=>!!t.api.manifest,5000, 'manifest');
  await t.api.start();
  assert.match(t.el('status').textContent,/Готово, SHA-256 совпадает/,'status '+t.el('status').textContent+'\n'+t.el('log').textContent);
  return t.api.metrics;
}
async function close(t) {
  for(const p of t.api.peers.values())p.rtc.close();
  // WebSocket is a lexical global in client module, cannot clean up through API; process will end.
}
async function run() {
  const manifest=await (await fetch('http://127.0.0.1:8765/manifest.json')).json();
  const baseline=await (await fetch('http://127.0.0.1:8765/stats')).json();
  const a=tab('A');
  await until(()=>a.el('log').textContent.includes('Подключено к указателю'),5000,'A joined').catch(e=>{console.error('A debug',a.el('log').textContent,a.output);throw e;});
  a.el('share').toggleShare(true);
  const am=await download(a);
  assert.equal(am.httpBytes,manifest.size);
  assert.equal(am.p2pBytes,0);
  await until(()=>a.api.cache.size===manifest.chunks.length, 5000, 'A all chunks');
  const b=tab('B');
  await until(()=>b.el('log').textContent.includes('Подключено к указателю'),5000,'B joined');
  await until(()=>b.api.metrics.peerCount>0,10000,'B channel established');
  const bm=await download(b);
  assert.equal(bm.p2pBytes,manifest.size,'B got all bytes from P2P');
  assert.equal(bm.httpBytes,0,'B did not fetch from origin');
  const afterB=await (await fetch('http://127.0.0.1:8765/stats')).json();
  assert.equal(afterB.origin_bytes-baseline.origin_bytes,manifest.size,'origin one full read for A only');
  // Third reader gets disjoint chunks from both seeders (if the runtime chooses both).
  b.el('share').toggleShare(true);
  await new Promise(r=>setTimeout(r,180));
  const c=tab('C');
  await until(()=>c.el('log').textContent.includes('Подключено к указателю'),5000,'C joined');
  await until(()=>c.api.metrics.peerCount===2,10000,'C connected to both seeders');
  const cm=await download(c);
  assert.equal(cm.httpBytes,0,'C downloads entirely from two peers');
  assert.equal(cm.p2pBytes,manifest.size);
  assert.ok(a.api.metrics.uploadBytes>manifest.size,'A also uploaded to C');
  assert.ok(b.api.metrics.uploadBytes>0,'B actually contributed chunks');
  // Poisoned peer data: an invalid piece must NEVER be delivered as valid.
  b.el('share').toggleShare(false);
  await new Promise(r=>setTimeout(r,180));
  const original = a.api.cache.get(0);
  a.api.cache.set(0, new Uint8Array(original.length));
  const d=tab('D');
  await until(()=>d.el('log').textContent.includes('Подключено к указателю'),5000,'D joined');
  await until(()=>d.api.metrics.peerCount>0,10000,'D connected');
  const dm=await download(d);
  assert.ok(dm.integrityFailures>=1,'D rejected poisoned P2P chunk');
  assert.equal(dm.httpBytes,manifest.chunks[0].size,'D fetched corrupted chunk from HTTP');
  assert.equal(dm.p2pBytes,manifest.size-manifest.chunks[0].size);
  a.api.cache.set(0, original);
  // A drops the data channel mid-piece; F must use HTTP for the missing data.
  const f=tab('F');
  await until(()=>f.el('log').textContent.includes('Подключено к указателю'),5000,'F joined');
  await until(()=>f.api.metrics.peerCount>0,10000,'F connected');
  const linkedChannel = [...f.api.peers.values()][0].channel.twin;
  const realSend = linkedChannel.send.bind(linkedChannel);
  let sentBytesToF = 0;
  linkedChannel.send = data => {
    realSend(data);
    if(typeof data !== 'string') {
      sentBytesToF += data.byteLength;
      if(sentBytesToF >= 5*16*1024) {
        linkedChannel.send = realSend;
        setImmediate(()=>linkedChannel.close());
      }
    }
  };
  const fm=await download(f);
  assert.ok(fm.peerFailures>=1,'F noticed broken peer connection');
  assert.ok(fm.httpBytes>0,'F resumed using HTTP after P2P dropped');
  // Consent revoked: last reader falls back to S3 without P2P.
  a.el('share').toggleShare(false);
  await new Promise(r=>setTimeout(r,200));
  const e=tab('E');
  await until(()=>e.el('log').textContent.includes('Подключено к указателю'),5000,'E joined');
  const em=await download(e);
  assert.equal(em.httpBytes,manifest.size,'E falls back to origin after consent revoked');
  assert.equal(em.p2pBytes,0);
  console.log(JSON.stringify({test:'simulated WebRTC + real WebSocket and HTTP',bytes:manifest.size,A:am,B:bm,C:cm,D:dm,F:fm,E:em,originAfterB:afterB.origin_bytes-baseline.origin_bytes}));
  await close(a);await close(b);await close(c);await close(d);await close(f);await close(e);
}
run().then(()=>process.exit(0)).catch(e=>{console.error(e.stack||e);process.exit(1)});