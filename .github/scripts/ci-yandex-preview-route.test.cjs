'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const {decision,safeDelta}=require('./ci-yandex-preview-route.cjs');
const b='aquilum-app/native/src/app/init/bootstrap.rs';
const m='aquilum-app/native/src/native_modules.rs';
const patch=lines=>['@@ -1 +1 @@',...lines].join('\n');
const yes=(files,diffs,reason)=>{const r=decision(files,diffs);assert.equal(r.heavy,true,reason);};
test('PDF-only integration seam avoids optional Windows preview',()=>{
  const diffs={[b]:patch(safeDelta[b]),[m]:patch(safeDelta[m]),'aquilum-app/native/src/pdf_worker.rs':patch(['+pub struct Worker;'])};
  assert.deepEqual(decision(Object.keys(diffs),diffs),{heavy:false,why:'reviewed PDF-only shared seam'});
});
test('owner changes, unknown and mixed changes fail closed',()=>{
 const p=patch(safeDelta[b]);const d={[b]:p};
 yes([b,'aquilum-app/native/src/yandex_probe_worker.rs'],{...d,'aquilum-app/native/src/yandex_probe_worker.rs':patch(['+pub fn probe() {}'])},'owner');
 yes([b,'aquilum-app/native/Cargo.toml'],{...d,'aquilum-app/native/Cargo.toml':patch(['+foo = true'])},'manifest');
 yes([b,'aquilum-app/native/src/app/settings/press.rs'],{...d,'aquilum-app/native/src/app/settings/press.rs':patch(['+abc'])},'shared');
 yes([b],{[b]:patch(['-            tree,','+            tree, yandex_probe: foo(),'])},'shared edit');
 yes([b],{},'diff unavailable');
 yes([b,'aquilum-app/native/src/pdf_worker.rs'],{...d,'aquilum-app/native/src/pdf_worker.rs':patch(['+yandex_s3_connection_preview()'])},'sensitive outside');
 yes([b,'.github/workflows/yandex-s3-native-preview.yml'],{...d,'.github/workflows/yandex-s3-native-preview.yml':patch(['+foo'])},'workflow');
});
test('no unknown file or empty changes can disable required proof',()=>{
 yes([],{},'empty');
 yes(['../evil'],{'../evil':patch(['+thing'])},'path traversal');
 yes(['aquilum-app/native/src/pdf_worker.rs'],{'aquilum-app/native/src/pdf_worker.rs':patch(['+foo'])},'no reviewed seam');
});
