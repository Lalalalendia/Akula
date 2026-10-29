'use strict';
const {classify}=require('./ci-rust-scope.cjs');

// Advisory ownership map. Do NOT use it as a GitHub job skip gate until the
// module is separately packaged and the required fallback tests are accepted.
const B0_CODE=new Set([
  'aquilum-app/core/src/server_bootstrap_lab.rs',
  'aquilum-app/core/tests/server_bootstrap_b0.rs',
  '.github/workflows/core-server-bootstrap-b0.yml',
]);
const B0_DOC='docs/platforms/AKULA-SERVER-BOOTSTRAP-B0.md';

function assessModuleImpact(paths){
  if(!Array.isArray(paths)||paths.length===0||paths.some(p=>
      typeof p!=='string'||!p||p.includes('\0')||p.startsWith('/')||p.includes('..'))){
    return {candidate:null,requiredScope:'workspace',maySkipRequiredChecks:false,
      reason:'invalid-or-missing-paths'};
  }
  const requiredScope=classify(paths);
  const executable=paths.filter(p=>!p.startsWith('docs/')&&p!=='README.md');
  const onlyB0=executable.length>0&&executable.every(p=>B0_CODE.has(p))&&
    paths.every(p=>B0_CODE.has(p)||p===B0_DOC||p.startsWith('docs/'));
  return {
    candidate:onlyB0?'server-bootstrap-b0':null,
    requiredScope,
    maySkipRequiredChecks:false,
    reason:onlyB0?'candidate-only-not-a-CI-gate':'mixed-shared-or-unrelated-files',
  };
}

module.exports={assessModuleImpact,B0_CODE};
