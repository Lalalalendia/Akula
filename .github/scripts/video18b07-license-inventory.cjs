'use strict';
const fs = require('node:fs');
const path = require('node:path');

const ROOTS = Object.freeze([
  {key: 'akula', name: 'aquilum-native'},
  {key: 'mux', name: 'akula-video-lite-lab'},
  {key: 'shiva', name: 'cli'},
]);

function reachablePackages(meta, rootName) {
  if (!meta || !Array.isArray(meta.packages) || !meta.resolve ||
      !Array.isArray(meta.resolve.nodes)) {
    throw new Error('VIDEO18B07: missing full Cargo metadata and dependency graph');
  }
  const pkgs = new Map(meta.packages.map(p => [p.id, p]));
  const graph = new Map(meta.resolve.nodes.map(n => [n.id, n]));
  const roots = meta.packages.filter(p => p.name === rootName && !p.source);
  if (roots.length !== 1) {
    throw new Error('VIDEO18B07: missing or ambiguous first-party crate ' + rootName);
  }
  const todo = [roots[0].id], visited = new Set();
  while (todo.length) {
    const id = todo.pop();
    if (visited.has(id)) continue;
    const pkg = pkgs.get(id), node = graph.get(id);
    if (!pkg || !node) throw new Error('VIDEO18B07: missing package/dependency node for ' + id);
    visited.add(id);
    const edges = Array.isArray(node.deps) ?
      node.deps.map(d => d.pkg) : (node.dependencies || []);
    for (const next of edges) {
      if (!pkgs.has(next) || !graph.has(next)) {
        throw new Error('VIDEO18B07: graph edge to unresolved dependency');
      }
      todo.push(next);
    }
  }
  return [...visited].map(id => pkgs.get(id)).sort((a,b) =>
    a.name.localeCompare(b.name) ||
    a.version.localeCompare(b.version) ||
    String(a.source || '').localeCompare(String(b.source || '')));
}

function componentPackage(pkg) {
  const license = String(pkg.license || '').trim();
  const file = String(pkg.license_file || '').trim();
  const status = !license && !file ? 'BLOCKED_MISSING_LICENSE_METADATA' :
    license ? 'REVIEW_SPDX_AND_LICENSE_NOTICE' : 'REVIEW_LICENSE_FILE_TEXT';
  return {
    name: pkg.name, version: pkg.version,
    source: pkg.source || 'local-source',
    license_expression: license || null,
    license_file_name: file ? path.basename(file) : null,
    review_status: status,
  };
}

function makeInventory(metas, allow, exactHead) {
  if (!/^[0-9a-f]{40}$/.test(exactHead)) throw new Error('VIDEO18B07: exact source SHA missing');
  const expectedExecutables = [
    'akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe',
    'akula-yt-dlp.exe','akula-quickjs.exe',
  ];
  if (!allow || allow.release_admitted !== false ||
      !Array.isArray(allow.components) || allow.components.length !== 5 ||
      allow.components.some((p,i) => p.name !== expectedExecutables[i]) ||
      allow.components.some(p => !/^(REVIEW|BLOCKED)/.test(p.license_status))) {
    throw new Error('VIDEO18B07: video release HOLD or exact five-component list missing');
  }
  const workspaces = {};
  for (const {key, name} of ROOTS) {
    const packages = reachablePackages(metas[key], name).map(componentPackage);
    workspaces[key] = {
      root: name,
      packages,
      count: packages.length,
      missing_metadata: packages.filter(x => x.review_status === 'BLOCKED_MISSING_LICENSE_METADATA').length,
    };
  }
  const seen = new Map();
  for (const {key} of ROOTS) for (const item of workspaces[key].packages) {
    const k = [item.name, item.version, item.source].join('\u0001');
    if (!seen.has(k)) seen.set(k, {...item,workspaces: []});
    const entry = seen.get(k);
    if (entry.license_expression !== item.license_expression) {
      throw new Error('VIDEO18B07: conflicting licenses for one immutable dependency ID');
    }
    entry.workspaces.push(key);
  }
  const all = [...seen.values()].sort((a,b) =>
    a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  return {
    schema: 1,
    title: 'VIDEO-18b-07 Rust transitive package license inventory - NON-RELEASE',
    exact_akula_head: exactHead,
    shiva_source_pin: 'cb542d30d2c18a18b1ee457f5158d5b0f551757e',
    release_admitted: false,
    legal_approval: false,
    sbom_is_license_attestation: false,
    review_hold: 'Manual license text, SPDX exceptions, binary assets and NOTICE obligations unresolved',
    roots: workspaces,
    package_count_deduplicated: all.length,
    missing_license_metadata: all.filter(x => x.review_status === 'BLOCKED_MISSING_LICENSE_METADATA'),
    unique_packages: all,
    bundled_upstream_executables: allow.components
      .filter(x => x.kind === 'pinned-upstream')
      .map(x => ({
        name: x.name,sha256: x.sha256,
        license_expression: x.license,
        review_status: x.license_status,
      })),
  };
}

function main() {
  if (process.argv.length !== 8) {
    throw new Error('Usage: video18b07-license-inventory.cjs <akula.json> <mux.json> <shiva.json> <allowlist.json> <exact_sha> <report.json>');
  }
  const [, , akulaPath, muxPath, shivaPath, allowPath, exactSha, reportPath] = process.argv;
  const read = p => JSON.parse(fs.readFileSync(p,'utf8'));
  const result = makeInventory({
    akula: read(akulaPath), mux: read(muxPath), shiva: read(shivaPath),
  },read(allowPath),exactSha);
  fs.mkdirSync(path.dirname(reportPath),{recursive: true});
  fs.writeFileSync(reportPath,JSON.stringify(result,null,2)+'\n','utf8');
  console.log('VIDEO18B07_LICENSE_INVENTORY_PASS akula=' + result.roots.akula.count +
    ' mux=' + result.roots.mux.count + ' shiva=' + result.roots.shiva.count +
    ' distinct=' + result.package_count_deduplicated +
    ' missing=' + result.missing_license_metadata.length +
    ' release_admitted=0');
}
if (require.main === module) main();
module.exports = {reachablePackages, componentPackage, makeInventory};
