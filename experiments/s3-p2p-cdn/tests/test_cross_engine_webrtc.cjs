// Real Chromium <-> Firefox DataChannel, availability, split swarm and hostile chunk.
// Both browsers run on one GitHub runner. This is NOT NAT/WAN certification.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium, firefox } = require('playwright');

const BASE = process.env.LAB_URL || 'http://127.0.0.1:8765';
const TIMEOUT = 120000;
const outputDir = path.join(__dirname, '..', 'test-results');
fs.mkdirSync(outputDir, { recursive: true });
const peers = [];
const logs = new Map();

async function waitUntil(page, fn, timeout = 40000) {
  await page.waitForFunction(fn, undefined, { timeout });
}

async function openPeer(browser, name) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const messages = [];
  page.on('console', msg => messages.push(`console:${msg.type()}: ${msg.text()}`));
  page.on('pageerror', error => messages.push(`pageerror: ${error.message}`));
  logs.set(name, messages);
  const peer = { context, page, name };
  peers.push(peer);
  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  await waitUntil(page, () => window.__akula?.manifest && document.querySelector('#log')?.textContent.includes('Подключено к указателю'));
  return peer;
}

async function download(peer) {
  const before = await peer.page.evaluate(() => window.__akula.metrics);
  await peer.page.locator('#download').click();
  await waitUntil(peer.page, () => /^(Готово, SHA-256 совпадает|Ошибка:)/.test(document.querySelector('#status').textContent), TIMEOUT);
  const status = await peer.page.locator('#status').textContent();
  assert.match(status, /Готово, SHA-256 совпадает/, `${peer.name}: ${status}`);
  const after = await peer.page.evaluate(() => window.__akula.metrics);
  return Object.fromEntries(['httpBytes', 'p2pBytes', 'uploadBytes', 'fromHttp', 'fromPeers', 'integrityFailures', 'peerFailures']
    .map(k => [k, after[k] - before[k]]));
}

async function inspectIce(peer) {
  return await peer.page.evaluate(async () => {
    const result = [];
    for (const item of window.__akula.peers.values()) {
      if (item.channel?.readyState !== 'open') continue;
      const stats = await item.rtc.getStats();
      let pair;
      for (const stat of stats.values()) {
        if (stat.type === 'transport' && stat.selectedCandidatePairId) {
          pair = stats.get(stat.selectedCandidatePairId);
          break;
        }
        if (stat.type === 'candidate-pair' && stat.state === 'succeeded' && stat.nominated) pair = stat;
      }
      const local = pair?.localCandidateId ? stats.get(pair.localCandidateId) : undefined;
      const remote = pair?.remoteCandidateId ? stats.get(pair.remoteCandidateId) : undefined;
      result.push({ connection: item.rtc.connectionState, channel: item.channel.readyState,
        localCandidateType: local?.candidateType || 'unknown', remoteCandidateType: remote?.candidateType || 'unknown',
        roundTripTime: pair?.currentRoundTripTime ?? null });
    }
    return result;
  });
}

async function setShare(peer, share) {
  const box = peer.page.locator('#share');
  if (share) await box.check();
  else await box.uncheck();
}

async function run() {
  const chromiumBrowser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
  const firefoxBrowser = await firefox.launch({ headless: true });
  try {
    const manifest = await (await fetch(`${BASE}/manifest.json`)).json();
    assert.equal(manifest.size, 8 * 1024 * 1024);
    assert.equal(manifest.chunks.length, 32);
    const origin0 = await (await fetch(`${BASE}/stats`)).json();

    // A downloads from source. Firefox B then obtains the exact object over
    // a real cross-engine RTCDataChannel without another full source download.
    const A = await openPeer(chromiumBrowser, 'Chromium-A');
    await setShare(A, true);
    const a = await download(A);
    assert.equal(a.httpBytes, manifest.size);
    const B = await openPeer(firefoxBrowser, 'Firefox-B');
    await waitUntil(B.page, () => window.__akula.metrics.peerCount > 0, 60000);
    const iceB = await inspectIce(B);
    const b = await download(B);
    assert.equal(b.httpBytes + b.p2pBytes, manifest.size);
    assert.ok(b.p2pBytes > 0, 'Firefox must obtain checked data over Chromium WebRTC');
    assert.ok(b.httpBytes < manifest.size, 'Firefox must save source traffic');

    // Enforce disjoint chunk sets on A and B to prove a third Chromium browser
    // has to consume bytes from BOTH peers, not merely select one fast peer.
    await setShare(B, true);
    for (const [p, remainder] of [[A, 0], [B, 1]]) {
      await p.page.evaluate(modulo => {
        for (const n of window.__akula.cache.keys()) {
          if ((n % 2) !== modulo) window.__akula.cache.delete(n);
        }
      }, remainder);
      await setShare(p, false);
      await setShare(p, true);
    }
    const C = await openPeer(chromiumBrowser, 'Chromium-C');
    await waitUntil(C.page, () => {
      const live = [...window.__akula.peers.values()].filter(x => x.channel?.readyState === 'open');
      if (live.length < 2) return false;
      const union = new Set(live.flatMap(x => [...x.have]));
      return live.some(x => x.have.size === 16) && union.size === 32;
    }, 60000);
    const beforeA = await A.page.evaluate(() => window.__akula.metrics.uploadBytes);
    const beforeB = await B.page.evaluate(() => window.__akula.metrics.uploadBytes);
    const c = await download(C);
    const afterA = await A.page.evaluate(() => window.__akula.metrics.uploadBytes);
    const afterB = await B.page.evaluate(() => window.__akula.metrics.uploadBytes);
    assert.equal(c.httpBytes + c.p2pBytes, manifest.size);
    assert.equal(c.p2pBytes, manifest.size, 'C must obtain all disjoint verified chunks from A and B');
    assert.ok(afterA > beforeA, 'Chromium A must upload some chunks to C');
    assert.ok(afterB > beforeB, 'Firefox B must upload some chunks to C');

    // Real DataChannel hostile-byte scenario: A2 seeds a verified object then
    // flips one cached byte AFTER verification. Firefox D must reject that
    // single chunk and get it from the HTTP origin, never output bad bytes.
    for (const peer of [A, B, C]) await peer.context.close();
    const A2 = await openPeer(chromiumBrowser, 'Chromium-Malicious-A2');
    await setShare(A2, true);
    assert.equal((await download(A2)).httpBytes, manifest.size);
    await A2.page.evaluate(() => { window.__akula.cache.get(0)[0] ^= 0xff; });
    const D = await openPeer(firefoxBrowser, 'Firefox-D-Verified');
    await waitUntil(D.page, () => window.__akula.metrics.peerCount > 0, 60000);
    const d = await download(D);
    assert.equal(d.httpBytes + d.p2pBytes, manifest.size);
    assert.ok(d.integrityFailures >= 1, 'Firefox must reject corrupted peer chunk');
    assert.ok(d.httpBytes >= manifest.chunks[0].size, 'HTTP must restore the rejected bytes');
    assert.ok(d.p2pBytes > 0, 'Uncorrupted chunks must still come from peer');

    const origin1 = await (await fetch(`${BASE}/stats`)).json();
    const summary = {
      scenario: 'REAL_CHROMIUM_FIREFOX_SPLIT_SWARM_BAD_PEER', bytes: manifest.size,
      a, b, c, d,
      chromium_uploaded_to_C: afterA - beforeA, firefox_uploaded_to_C: afterB - beforeB,
      firefox_candidate_types: iceB,
      originBytesTotal: origin1.origin_bytes - origin0.origin_bytes,
      passed: true,
      limitations: 'Both browser engines on same physical GitHub runner and localhost; not WAN/NAT, real S3, Safari or mobile.',
    };
    fs.writeFileSync(path.join(outputDir, 'cross-engine-result.json'), JSON.stringify(summary, null, 2));
    console.log(JSON.stringify(summary, null, 2));
  } catch (error) {
    for (const peer of peers) {
      try { await peer.page.screenshot({ path: path.join(outputDir, `${peer.name}-failure.png`), fullPage: true }); } catch {}
      try {
        const ui = await peer.page.locator('#log').textContent();
        fs.writeFileSync(path.join(outputDir, `${peer.name}-log.txt`), `${(logs.get(peer.name) || []).join('\n')}\n${ui}`);
      } catch {}
    }
    throw error;
  } finally {
    await firefoxBrowser.close();
    await chromiumBrowser.close();
  }
}

run().catch(error => { console.error(error.stack || error); process.exitCode = 1; });