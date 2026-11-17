'use strict';
const fs = require('node:fs');
const path = require('node:path');
const {reachablePackages} = require('./video18b07-license-inventory.cjs');

const TARGET = 'x86_64-pc-windows-msvc';
const ROOTS = Object.freeze([
  {key:'akula',name:'aquilum-native'},
  {key:'mux',name:'akula-video-lite-lab'},
  {key:'shiva',name:'cli'},
]);
const EXECUTABLES = Object.freeze([
  'akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
  'akula-yt-dlp.exe','akula-quickjs.exe',
]);
function identity(pkg) {
  return [pkg.name,pkg.version,pkg.source || 'local-source'].join('\u0001');
}
function buildTriage(targetMetas,conservative,evidence,allowlist,head) {
  if(!/^[0-9a-f]{40}$/.test(head) ||
     conservative?.exact_akula_head !== head ||
     evidence?.source_head !== head ||
     conservative.release_admitted !== false ||
     conservative.legal_approval !== false ||
     evidence.release_admitted !== false ||
     evidence.legal_approval !== false ||
     evidence.license_text_review_complete !== false) {
    throw Error('VIDEO18B09: invalid source identity or release HOLD');
  }
  if(allowlist?.release_admitted !== false ||
     allowlist.installer_signing_authorized !== false ||
     allowlist.stage_binary_distribution_authorized !== false ||
     JSON.stringify(allowlist.components?.map(c=>c.name)) !== JSON.stringify(EXECUTABLES) ||
     allowlist.components[2].license !== 'AGPL-3.0-only' ||
     allowlist.components.some(c=>!String(c.license_status).startsWith('REVIEW'))) {
    throw Error('VIDEO18B09: five-EXE release HOLD/allowlist mismatch');
  }
  if(!Array.isArray(conservative.unique_packages) ||
     !Array.isArray(evidence.packages) ||
     evidence.packages.length !== evidence.package_count ||
     !conservative.roots) {
    throw Error('VIDEO18B09: missing exact conservative/evidence graph');
  }
  const texts = new Map();
  for(const p of evidence.packages) {
    const id=identity(p);
    if(texts.has(id)) throw Error('VIDEO18B09: duplicate license-file evidence');
    if(!['TEXT_FOUND_REQUIRES_REVIEW','TEXT_NOT_VERIFIED'].includes(p.status)) throw Error('VIDEO18B09: invalid evidence classification');
    texts.set(id,p);
  }
  const complete = new Set(conservative.unique_packages.map(identity));
  if(complete.size!==conservative.unique_packages.length ||
     texts.size!==complete.size ||
     [...texts.keys()].some(id=>!complete.has(id))) {
    throw Error('VIDEO18B09: source/evidence package mismatch');
  }
  const targetRoots = {};
  const found = new Map();
  for(const {key,name} of ROOTS) {
    const base = conservative.roots[key];
    if(!base || base.root !== name || !Array.isArray(base.packages)) {
      throw Error('VIDEO18B09: mismatched conservative root '+key);
    }
    const baseSet = new Set(base.packages.map(identity));
    const selected = reachablePackages(targetMetas[key],name);
    if(selected.length===0 || selected.length > baseSet.size) throw Error('VIDEO18B09: invalid target closure size');
    const missing = [];
    for(const pkg of selected) {
      const id=identity(pkg);
      if(!baseSet.has(id) || !complete.has(id) || !texts.has(id)) {
        throw Error('VIDEO18B09: target package not in pinned full dependency inventory '+pkg.name);
      }
      const file=texts.get(id);
      const item=found.get(id)||{
        name:pkg.name,version:pkg.version,source:pkg.source||'local-source',
        license_expression:file.license_expression,
        license_file_status:file.status,
        roots:[],
      };
      item.roots.push(key);
      found.set(id,item);
      if(file.status==='TEXT_NOT_VERIFIED') missing.push(pkg.name+'@'+pkg.version);
    }
    targetRoots[key]={
      name, packages: selected.length, conservative_packages:baseSet.size,
      excluded_from_conservative:baseSet.size-selected.length,
      text_not_verified:missing.length,
    };
  }
  const packages=[...found.values()].sort((a,b)=>
    a.name.localeCompare(b.name)||a.version.localeCompare(b.version)||a.source.localeCompare(b.source));
  const needsText=packages.filter(p=>p.license_file_status==='TEXT_NOT_VERIFIED');
  // Priorities are review order, NOT evidence of a legal violation.
  for(const p of needsText) {
    p.review_priority=p.source==='local-source'?'FIRST_PARTY_DOCUMENT':
      p.roots.includes('mux')||p.roots.includes('shiva')?'RUNTIME_COMPONENT_DEPENDENCY':'APPLICATION_DEPENDENCY';
  }
  return {
    schema:1,
    title:'VIDEO-18b-09 Windows target-specific license triage (non-release)',
    exact_akula_head:head,
    shiva_source_pin:conservative.shiva_source_pin,
    target:TARGET,
    akula_feature_set:['installed','video-rust-bundle'],
    selected_roots:targetRoots,
    conservative_unique_packages:complete.size,
    target_unique_packages:packages.length,
    excluded_from_conservative:complete.size-packages.length,
    license_text_not_verified:needsText.length,
    license_text_located_but_unreviewed:packages.length-needsText.length,
    priority_review_queue:needsText,
    included_package_summary:packages,
    bundled_executables:allowlist.components.map(c=>({name:c.name,license_status:c.license_status})),
    release_admitted:false,legal_approval:false,
    sbom_is_release_attestation:false,
    license_text_review_complete:false,
    binary_asset_notices_review_complete:false,
    notes:'Cargo metadata --filter-platform narrows resolve graph, not an actual linked PE SBOM. All selected package texts/NOTICE and bundled yt-dlp/QuickJS assets still require human review.',
  };
}
function main() {
  if(process.argv.length !== 10) throw Error('Usage: script win-akula.json win-mux.json win-shiva.json full.json evidence.json allowlist.json SHA out.json');
  const [,,ak,mx,sh,full,ev,al,head,out]=process.argv;
  const read=p=>JSON.parse(fs.readFileSync(p,'utf8'));
  const result=buildTriage({akula:read(ak),mux:read(mx),shiva:read(sh)},read(full),read(ev),read(al),head);
  fs.mkdirSync(path.dirname(out),{recursive:true});
  fs.writeFileSync(out,JSON.stringify(result,null,2)+'\n');
  console.log('VIDEO18B09_WINDOWS_TARGET_TRIAGE_PASS target='+result.target+
    ' distinct='+result.target_unique_packages+
    ' unresolved_text='+result.license_text_not_verified+
    ' release_admitted=0');
}
if(require.main===module)main();
module.exports={buildTriage,identity,TARGET,EXECUTABLES};
