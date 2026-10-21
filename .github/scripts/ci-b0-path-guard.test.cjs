'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const B0='.github/workflows/core-server-bootstrap-b0.yml';
test('Bootstrap B0 path scope keeps direct changes and lockfile',()=>{
 const s=fs.readFileSync(B0,'utf8');
 for(const path of ['modules/bootstrap-b0/**','aquilum-app/core/src/server_bootstrap_lab.rs','aquilum-app/core/tests/server_bootstrap_b0.rs','aquilum-app/core/Cargo.lock'])assert.ok(s.includes(path),path);
 assert.ok(!s.includes('      - "aquilum-app/core/Cargo.toml"'));
 assert.ok(!s.includes('      - "aquilum-app/core/src/lib.rs"'));
 assert.ok(s.includes('workflow_dispatch:'));
});
