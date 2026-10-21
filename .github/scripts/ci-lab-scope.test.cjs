'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {isolated}=require('./ci-lab-scope.cjs');
const C='aquilum-app/core/Cargo.toml',L='aquilum-app/core/src/lib.rs';
const files=[{status:'M',path:C},{status:'M',path:L},
{status:'A',path:'aquilum-app/core/src/notion_api_probe.rs'},
{status:'A',path:'aquilum-app/core/src/notion_api_probe/http.rs'},
{status:'A',path:'aquilum-app/core/src/bin/akula_notion_api_probe.rs'},
{status:'A',path:'.github/workflows/notion-api-probe.yml'}];
const cargoOld='[features]\ndefault = []\nnode-lab = []\n[dependencies]\nserde = "1"\n';
const cargoNew='[features]\ndefault = []\nnode-lab = []\nnotion-api-probe = []\n[dependencies]\nserde = "1"\n[[bin]]\nname = "akula-notion-api-probe"\npath = "src/bin/akula_notion_api_probe.rs"\nrequired-features = ["notion-api-probe"]\n';
const cargoDiff='@@ -3,0 +4 @@\n+notion-api-probe = []\n@@ -6,0 +7,5 @@\n+\n+[[bin]]\n+name = "akula-notion-api-probe"\n+path = "src/bin/akula_notion_api_probe.rs"\n+required-features = ["notion-api-probe"]\n';
const libDiff='@@ -8,0 +9,2 @@\n+#[cfg(feature = "notion-api-probe")]\n+pub mod notion_api_probe;\n';
const base={files,cargoOld,cargoNew,cargoDiff,libDiff};
const verify=change=>isolated({...base,...change});
test('skip only when a new gated optional module is isolated',()=>assert.equal(verify({}),true));
test('unconditional lib export runs every lab',()=>assert.equal(verify({libDiff:'+pub mod notion_api_probe;\n'}),false));
test('changed dependency runs all labs',()=>assert.equal(verify({cargoDiff:cargoDiff+'+serde = "2"\n'}),false));
test('removed dependency runs all labs',()=>assert.equal(verify({cargoDiff:cargoDiff+'-serde = "1"\n'}),false));
test('feature default activation runs labs',()=>assert.equal(verify({cargoNew:cargoNew.replace('default = []','default = ["notion-api-probe"]')}),false));
test('modified bootstrap source runs labs',()=>assert.equal(verify({files:[...files,{status:'M',path:'aquilum-app/core/src/server_bootstrap_lab.rs'}]}),false));
test('modified Cargo.lock runs labs',()=>assert.equal(verify({files:[...files,{status:'M',path:'aquilum-app/Cargo.lock'}]}),false));
test('new unrelated sync source runs labs',()=>assert.equal(verify({files:[...files,{status:'A',path:'aquilum-app/core/src/sync/unsafe.rs'}]}),false));
test('missing required feature runs labs',()=>assert.equal(verify({cargoDiff:cargoDiff.replace('+required-features = ["notion-api-probe"]\n','')}),false));
test('non-probe feature runs labs',()=>assert.equal(verify({cargoDiff:cargoDiff.replace('notion-api-probe','notion-core')}),false));
test('changed target workflow runs labs',()=>assert.equal(verify({files:[...files,{status:'M',path:'.github/workflows/core-server-bootstrap-b0.yml'}]}),false));
test('renames/deletes fail closed',()=>assert.equal(verify({files:[...files,{status:'R',path:'aquilum-app/core/src/foo.rs'}]}),false));
