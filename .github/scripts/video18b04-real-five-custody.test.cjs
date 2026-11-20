'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const root = path.resolve(__dirname, '../..');
const read = p => fs.readFileSync(path.join(root, p), 'utf8');
const allowPath = 'docs/release/video18b04-five-exe-allowlist.json';
const testPath = '.github/scripts/video18b04-real-five-custody.test.cjs';
const scriptPath = 'scripts/windows-video18b04-real-five-custody.ps1';
const workflowPath = '.github/workflows/akula-video18b04-real-five-custody.yml';
const docPath = 'docs/release/video18b04-real-five-custody.md';
const names = ['akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe'];
const pins = {
  'akula-yt-dlp.exe': ['2026.10.26','f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d',23575744,'GPL-3.0-or-later'],
  'akula-quickjs.exe': ['v0.18.0','ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7',1873152,'MIT']
};
function validate(p, ps, workflow, reference) {
  assert.equal(p.schema, 1);
  assert.equal(p.release_admitted, false,'HOLD: release must not become admitted');
  assert.equal(p.installer_signing_authorized, false,'HOLD: signer must never be used in PR lab');
  assert.equal(p.stage_binary_distribution_authorized, false,'HOLD: do not upload unsigned EXEs');
  assert.deepEqual(p.supported_windows, ['10','11']);
  assert.deepEqual(p.components.map(x=>x.name), names,'HOLD: exactly five components, fixed order');
  for (const c of p.components) {
    assert.ok(['source-built','pinned-upstream'].includes(c.kind));
    assert.ok(/^(REVIEW|BLOCKED)/.test(c.license_status),'HOLD: no unsupported license approval');
    assert.ok(c.license_notice && c.license,'HOLD: missing license provenance');
    if (c.kind === 'source-built') {
      assert.ok(c.lock_path && c.source_path,'HOLD: unpinned source-built dependency lock');
    }
  }
  const shiva = p.components[1];
  assert.equal(shiva.reference,'cb542d30d2c18a18b1ee457f5158d5b0f551757e');
  assert.equal(shiva.lock_sha256,'62b83af03e4079f6603eef7f7170edc4138a8233ad57829adf13baacd3f2b1d4');
  assert.equal(shiva.checkout_crlf_sha256,'80a05fc499cdc6961256d4b1569db828d6a5e56506aa37af60ac25c0d196c9f1');
  assert.equal(shiva.git_blob_sha1,'e9d6d1feee9ef3c76df19ce24033e6f34f0181d6');
  assert.equal(shiva.license_status,'REVIEW_TRANSITIVE_DEPENDENCY_NOTICES');
  assert.ok(shiva.license.includes('MIT OR Apache-2.0'),'HOLD: Shiva license metadata was not repaired');
  assert.equal(p.components[2].license_status,'REVIEW_TRANSITIVE_DEPENDENCY_NOTICES');
  assert.equal(p.components[2].license,'AGPL-3.0-only');
  assert.equal(p.components[3].source_license,'Unlicense');
  assert.equal(p.components[3].license_status,'REVIEW_BUNDLED_DEPENDENCIES');
  assert.ok(p.components[3].license_notice.endsWith('/THIRD_PARTY_LICENSES.txt'),'HOLD: bundled yt-dlp notices lost');
  for (const c of p.components.filter(x=>x.kind==='pinned-upstream')) {
    const pin = pins[c.name];
    assert.ok(pin,'HOLD: unexpected upstream EXE');
    assert.equal(c.reference,pin[0]);
    assert.equal(c.sha256,pin[1]);
    assert.equal(c.size_bytes,pin[2]);
    assert.equal(c.license,pin[3]);
    assert.ok(c.source_url.startsWith('https://github.com/'+c.repository+'/releases/download/'),'HOLD: untrusted upstream host');
  }
  assert.ok(reference.includes(pins['akula-yt-dlp.exe'][1]),'HOLD: yt-dlp reference does not match native pin');
  assert.ok(reference.includes(pins['akula-quickjs.exe'][1]),'HOLD: QuickJS reference does not match native pin');
  for (const term of [
    "RUNNER_ENVIRONMENT -cne 'github-hosted'",
    "GITHUB_EVENT_NAME -cne 'pull_request'",
    'EXPECTED_HEAD_SHA','Reject-Reparse','Check-Pe',
    'Get-AuthenticodeSignature',
    'Check-Pe $target',
    'VIDEO18B05_SHIVA_LOCK_PIN_PASS',
    'git -C $shivaDir ls-files --error-unmatch -- Cargo.lock',
    '$shivaCheckoutHash -cne $expectedShivaLock -and',
    '$shivaCheckoutHash -cne $expectedShivaCrlf',
    '$actualBlob -cne $expectedBlob',
    'AKULA_BUNDLED_MUX_SHA256',
    'VIDEO18B04_TAMPER_REFUSAL_PASS',
    'VIDEO18B04_LICENSE_RELEASE_HOLD_PASS',
    'VIDEO18B04_REAL_FIVE_CUSTODY_PASS',
    'Remove-Item -LiteralPath $stage -Recurse'
  ]) assert.ok(ps.includes(term),'HOLD: missing real custody verification '+term);
  assert.ok(!/cargo packager signer|gh release create|publish_beta|WriteRegStr/i.test(ps),'HOLD: lab cannot release or install');
  const header = workflow.split(/\r?\npermissions:/,1)[0];
  assert.match(header,/^on:\r?\n  pull_request:\r?\n    branches: \[main\]/m);
  for (const p of [allowPath,testPath,scriptPath,workflowPath,docPath]) {
    assert.ok(header.includes("      - '"+p+"'"),'HOLD: missing custody PR source trigger '+p);
  }
  assert.match(workflow,/concurrency:\s*\n  group:/);
  assert.match(workflow,/runs-on: windows-2022/);
  assert.match(workflow,/persist-credentials: false/);
  assert.match(workflow,/retention-days: 1/);
  assert.match(workflow,/shiva_commit/i);
  assert.ok(workflow.includes('cb542d30d2c18a18b1ee457f5158d5b0f551757e'),'HOLD: workflow is not pinned to repaired Shiva');
  assert.ok(workflow.includes('cargo build --release --locked --manifest-path _shiva-preview/Cargo.toml -p cli'),'HOLD: Shiva must build from immutable lock');
  assert.ok(workflow.includes('62b83af03e4079f6603eef7f7170edc4138a8233ad57829adf13baacd3f2b1d4'),'HOLD: missing pinned LF lock hash check');
  assert.ok(workflow.includes('80a05fc499cdc6961256d4b1569db828d6a5e56506aa37af60ac25c0d196c9f1'),'HOLD: missing pinned CRLF checkout hash check');
  assert.ok(workflow.includes('e9d6d1feee9ef3c76df19ce24033e6f34f0181d6'),'HOLD: missing exact git source blob check');
  assert.match(workflow,/video18b04-five-exe-allowlist\.json/);
  assert.ok(!workflow.includes('workflow_dispatch:'),'HOLD: no manual publishing entrypoint');
  assert.ok(!/actions\/upload-artifact[\s\S]*\.(exe|zip)/i.test(workflow),'HOLD: never upload unsigned EXEs');
}
const inputs = () => [JSON.parse(read(allowPath)),read(scriptPath),read(workflowPath),read('aquilum-app/native/src/video/light.rs')];
test('VIDEO18B04 five-component custody is immutable and keeps release HOLD', () => validate(...inputs()));
test('VIDEO18B04 refuses silently enabling the video installer', () => {
  const [p,ps,wf,ref]=inputs();p.release_admitted=true;
  assert.throws(()=>validate(p,ps,wf,ref),/HOLD/);
});
test('VIDEO18B04 refuses altered upstream SHA256', () => {
  const [p,ps,wf,ref]=inputs();p.components[3].sha256='a'.repeat(64);
  assert.throws(()=>validate(p,ps,wf,ref));
});
test('VIDEO18B04 refuses dropping Shiva license blocker', () => {
  const [p,ps,wf,ref]=inputs();p.components[1].license_status='APPROVED';
  assert.throws(()=>validate(p,ps,wf,ref),/HOLD/);
});
test('VIDEO18B04 refuses adding a sixth unverified executable', () => {
  const [p,ps,wf,ref]=inputs();p.components.push({...p.components[0],name:'untrusted.exe'});
  assert.throws(()=>validate(p,ps,wf,ref),/HOLD/);
});
test('VIDEO18B04 refuses missing workflow trigger', () => {
  const [p,ps,wf,ref]=inputs();const changed=wf.replace("      - '"+allowPath+"'\n",'');
  assert.notEqual(changed,wf);
  assert.throws(()=>validate(p,ps,changed,ref),/HOLD/);
});
test('VIDEO18B04 refuses disabled real custody PE test', () => {
  const [p,ps,wf,ref]=inputs();const changed=ps.replace('Check-Pe $target','');
  assert.notEqual(changed,ps);
  // In addition to the positive presence, prevent invalid stage mutation.
  assert.throws(()=>validate(p,changed,wf,ref),/HOLD/);
});

test('VIDEO18B05 rejects changing the committed Shiva Cargo.lock SHA256', () => {
  const [p,ps,wf,ref]=inputs();
  p.components[1].lock_sha256='0'.repeat(64);
  assert.throws(()=>validate(p,ps,wf,ref),/HOLD|AssertionError/);
});
test('VIDEO18B05 rejects switching Shiva CI build to unlocked dependency resolution', () => {
  const [p,ps,wf,ref]=inputs();
  const changed=wf.replace('cargo build --release --locked --manifest-path _shiva-preview/Cargo.toml -p cli','cargo build --release --manifest-path _shiva-preview/Cargo.toml -p cli');
  assert.notEqual(changed,wf);
  assert.throws(()=>validate(p,ps,changed,ref),/HOLD/);
});

test('VIDEO18B05 rejects an unauthorized CRLF checkout digest', () => {
  const [p,ps,wf,ref]=inputs();
  p.components[1].checkout_crlf_sha256='0'.repeat(64);
  assert.throws(()=>validate(p,ps,wf,ref));
});
test('VIDEO18B05 rejects an unauthorized Shiva lock git blob', () => {
  const [p,ps,wf,ref]=inputs();
  p.components[1].git_blob_sha1='f'.repeat(40);
  assert.throws(()=>validate(p,ps,wf,ref));
});
