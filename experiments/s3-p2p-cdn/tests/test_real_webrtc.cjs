// Real Chromium RTCDataChannel E2E: no FakeRTCPeerConnection or protocol shims.
// Expected server: http://127.0.0.1:8765; never exposed to the Internet.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');

const BASE = process.env.LAB_URL || 'http://127.0.0.1:8765';
const TIMEOUT = 120000;
const resultsDir = path.join(__dirname, '..', 'test-results');
fs.mkdirSync(resultsDir, { recursive: true });
const pageLogs = new Map();

async function waitUntil(page, expression, timeout = 40000) {
  await page.waitForFunction(expression, undefined, { timeout });
}
async function createPeer(browser, label) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const messages = [];
  pageLogs.set(label, messages);
  page.on('console', msg => messages.push(`[console:${msg.type()}] ${msg.text()}`));
  page.on('pageerror', err => messages.push(`[pageerror] ${err.message}`));
  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  await waitUntil(page, () => window.__akula?.manifest && document.querySelector('#log')?.textContent.includes('Подключено к указателю'));
  return { context, page, label };
}
async function download(peer) {
  await peer.page.locator('#download').click();
  await waitUntil(peer.page, () => /^(Готово, SHA-256 совпадает|Ошибка:)/.test(document.querySelector('#status').textContent), TIMEOUT);
  const status = await peer.page.locator('#status').textContent();
  assert.match(status, /Готово, SHA-256 совпадает/, `${peer.label}: ${status}`);
  return await peer.page.evaluate(() => window.__akula.metrics);
}
async function stats() {
  const response = await fetch(`${BASE}/stats`);
  assert.equal(response.status, 200);
  return await response.json();
}
async function run() {
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
  const peers = [];
  try {
    const response = await fetch(`${BASE}/manifest.json`);
    assert.equal(response.status, 200);
    const manifest = await response.json();
    assert.equal(manifest.size, 8 * 1024 * 1024);
    const initial = await stats();

    const a = await createPeer(browser, 'A'); peers.push(a);
    await a.page.locator('#share').check();
    const A = await download(a);
    assert.equal(A.httpBytes, manifest.size, 'A should populate cache from HTTP');
    assert.equal(A.p2pBytes, 0);

    const b = await createPeer(browser, 'B'); peers.push(b);
    await waitUntil(b.page, () => window.__akula.metrics.peerCount > 0, 60000);
    const B = await download(b);
    assert.equal(B.httpBytes + B.p2pBytes, manifest.size);
    assert.ok(B.p2pBytes > 0, 'B must receive verified bytes through real WebRTC');
    assert.ok(B.httpBytes < manifest.size, 'B must save HTTP/origin traffic');
    const afterB = await stats();
    assert.ok(afterB.origin_bytes - initial.origin_bytes < 2 * manifest.size, 'Origin traffic must be lower than HTTP-only baseline');
    assert.ok(afterB.origin_bytes - initial.origin_bytes >= manifest.size, 'A received original data from HTTP');

    // Consent is a strict permission boundary: B never opted into uploading.
    await a.page.locator('#share').uncheck();
    await waitUntil(b.page, () => [...window.__akula.peers.values()].every(p => p.have.size === 0), 10000);
    const c = await createPeer(browser, 'C'); peers.push(c);
    const C = await download(c);
    assert.equal(C.httpBytes, manifest.size, 'C must fall back to original HTTP after opt-out');
    assert.equal(C.p2pBytes, 0);

    const summary = { scenario: 'REAL_CHROMIUM_WEBRTC', manifest_sha256: manifest.content_id, bytes: manifest.size,
      A, B, C, origin_after_B: afterB.origin_bytes - initial.origin_bytes,
      passed: true, limitations: 'same GitHub Actions runner, localhost, not different NAT or WAN' };
    fs.writeFileSync(path.join(resultsDir, 'real-webrtc-result.json'), JSON.stringify(summary, null, 2));
    process.stdout.write(JSON.stringify(summary, null, 2) + '\n');
  } catch (error) {
    for (const peer of peers) {
      try { await peer.page.screenshot({ path: path.join(resultsDir, `${peer.label}-failure.png`), fullPage: true }); } catch {}
      try { fs.writeFileSync(path.join(resultsDir, `${peer.label}-log.txt`), (pageLogs.get(peer.label) || []).join('\n') + '\n' + await peer.page.locator('#log').textContent()); } catch {}
    }
    throw error;
  } finally {
    await browser.close();
  }
}
run().catch(error => { console.error(error.stack || error); process.exitCode = 1; });