// True Chromium WebRTC + public-readable MinIO S3 object behind CI-local TLS.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const { chromium } = require('playwright');

const BASE = 'http://127.0.0.1:8765';
const METRICS = 'https://127.0.0.1:9443/metrics';
const TIMEOUT = 120000;
const RESULTS = path.join(__dirname, '..', 'test-results');
fs.mkdirSync(RESULTS, { recursive: true });

async function getMetrics() {
  const agent = new https.Agent({ rejectUnauthorized: false }); // ephemeral loopback TLS only
  const result = await new Promise((resolve, reject) => {
    https.get(METRICS, { agent }, res => {
      let bytes = '';
      res.on('data', chunk => { bytes += chunk; });
      res.on('end', () => {
        try { resolve(JSON.parse(bytes)); } catch (e) { reject(e); }
      });
      res.on('error', reject);
    }).on('error', reject);
  });
  return result;
}

async function openTab(browser, name, log) {
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await context.newPage();
  page.on('console', msg => log.push(`${name}: console ${msg.type()}: ${msg.text()}`));
  page.on('pageerror', e => log.push(`${name}: pageerror ${e.message}`));
  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => window.__akula?.manifest &&
    document.querySelector('#log')?.textContent.includes('Подключено к указателю'),
    undefined, { timeout: 30000 });
  return { context, page };
}

async function readFile(page) {
  const start = await page.evaluate(() => ({ ...window.__akula.metrics }));
  await page.locator('#download').click();
  await page.waitForFunction(() => /^(Готово, SHA-256 совпадает|Ошибка:)/.test(
    document.querySelector('#status').textContent), undefined, { timeout: TIMEOUT });
  const status = await page.locator('#status').textContent();
  assert.match(status, /Готово, SHA-256 совпадает/, status);
  const end = await page.evaluate(() => ({ ...window.__akula.metrics }));
  return Object.fromEntries(['httpBytes', 'p2pBytes', 'uploadBytes', 'fromHttp',
    'fromPeers', 'integrityFailures', 'peerFailures'].map(k => [k, end[k] - start[k]]));
}

async function run() {
  const logs = [], peers = [];
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
  try {
    const manifest = await (await fetch(`${BASE}/manifest.json`)).json();
    assert.match(manifest.origin_url, /^https:\/\/127\.0\.0\.1:9443\/akula-public-webrtc-/);
    assert.equal(manifest.size, 8 * 1024 * 1024);
    const before = await getMetrics();
    const A = await openTab(browser, 'A-seed', logs); peers.push(A);
    await A.page.locator('#share').check();
    const a = await readFile(A.page);
    const afterA = await getMetrics();
    assert.equal(a.httpBytes, manifest.size);
    assert.equal(a.p2pBytes, 0);
    assert.equal(afterA.origin_bytes - before.origin_bytes, manifest.size,
      'First browser must download full object from real MinIO via HTTPS');

    const B = await openTab(browser, 'B-peer', logs); peers.push(B);
    await B.page.waitForFunction(() => window.__akula.metrics.peerCount > 0,
      undefined, { timeout: 60000 });
    const b = await readFile(B.page);
    const afterB = await getMetrics();
    assert.equal(b.httpBytes, 0);
    assert.equal(b.p2pBytes, manifest.size);
    assert.equal(afterB.origin_bytes - afterA.origin_bytes, 0,
      'Second browser must NOT refetch from MinIO');

    await A.page.locator('#share').uncheck();
    const C = await openTab(browser, 'C-after-unshare', logs); peers.push(C);
    const c = await readFile(C.page);
    const afterC = await getMetrics();
    assert.equal(c.httpBytes, manifest.size);
    assert.equal(c.p2pBytes, 0);
    assert.equal(afterC.origin_bytes - afterB.origin_bytes, manifest.size,
      'Third browser must fallback to MinIO after upload consent withdrawn');

    const report = {
      scenario: 'REAL_MINIO_S3_HTTPS_TO_BROWSER_P2P_AND_FALLBACK',
      source: manifest.origin_url, fixtureBytes: manifest.size,
      contentId: manifest.content_id, a, b, c,
      minioOrigin: { firstBrowserBytes: afterA.origin_bytes - before.origin_bytes,
        secondBrowserAdditionalBytes: afterB.origin_bytes - afterA.origin_bytes,
        thirdBrowserAdditionalBytes: afterC.origin_bytes - afterB.origin_bytes },
      passed: true,
      limitations: 'Local disposable MinIO behind test TLS gateway; both Chromium browsers on one CI runner. NOT AWS S3, independent WAN/NAT/TURN or production TLS.',
    };
    fs.writeFileSync(path.join(RESULTS, 'minio-real-browser-result.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } catch (err) {
    for (let i=0; i<peers.length; i++) {
      try { await peers[i].page.screenshot({ path: path.join(RESULTS, `minio-failure-${i}.png`) }); } catch {}
    }
    fs.writeFileSync(path.join(RESULTS, 'minio-browser-debug.log'), logs.join('\n'));
    throw err;
  } finally {
    await Promise.allSettled(peers.map(x=>x.context.close()));
    await browser.close();
  }
}
run().catch(e => { console.error(e.stack || e); process.exitCode = 1; });
