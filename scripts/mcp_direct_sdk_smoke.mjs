#!/usr/bin/env node
/**
 * AKAI-CONF-03: official SDK -> real Akula HTTP/Core/File Gate, NO bearer proxy.
 * All documents, appdata, endpoint and credentials are disposable test fixtures.
 * Not proof of MCP 2026 or per-agent grant isolation.
 */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { request as rawHttpRequest } from "node:http";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const sdkRoot = process.env.AKAI_MCP_SDK_ROOT;
const portFile = process.env.AKAI_MCP_PORT_FILE;
const workspaceFile = process.env.AKAI_MCP_WORKSPACE_FILE;
const bearer = process.env.AKAI_MCP_TEST_BEARER;
assert.ok(sdkRoot && portFile && workspaceFile && bearer?.length >= 32);
const sdkUrl = name => pathToFileURL(resolve(
  sdkRoot, "node_modules", "@modelcontextprotocol", "sdk",
  "dist", "esm", "client", name
)).href;
const [{ Client }, { StreamableHTTPClientTransport }] = await Promise.all([
  import(sdkUrl("index.js")),
  import(sdkUrl("streamableHttp.js")),
]);

const port = (await readFile(portFile, "utf8")).trim();
assert.match(port, /^[0-9]{1,5}$/);
const workspace = (await readFile(workspaceFile, "utf8")).trim();
assert.ok(workspace.length > 0);
const endpoint = new URL("http://127.0.0.1:" + port + "/mcp");

function client(label) {
  const transport = new StreamableHTTPClientTransport(endpoint, {
    requestInit: { headers: { Authorization: "Bearer " + bearer } },
  });
  const sdk = new Client({ name: "akai-direct-" + label, version: "1.0.0" }, {
    capabilities: {},
  });
  return { sdk, transport };
}
function toolJson(response) {
  assert.equal(response.isError, false, "unexpected tool error: " + JSON.stringify(response.content));
  const text = response.content?.find(x => x.type === "text")?.text;
  assert.ok(typeof text === "string", "missing tool text result");
  return JSON.parse(text);
}
async function deniedWrite(headers = {}) {
  const request = {
    jsonrpc: "2.0", id: 901, method: "tools/call",
    params: { name: "update_note", arguments: {
      note: "Conformance.md", workspace, mode: "replace",
      content: "UNAUTHORIZED_OVERWRITE", open: false,
    }},
  };
  return fetch(endpoint, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Accept": "application/json, text/event-stream",
      ...headers,
    },
    body: JSON.stringify(request),
    signal: AbortSignal.timeout(5000),
  });
}

async function deniedHost(host) {
  const body = JSON.stringify({
    jsonrpc: "2.0", id: 990, method: "tools/call",
    params: { name: "update_note", arguments: {
      note: "Conformance.md", workspace, mode: "replace",
      content: "HOST_REBINDING_OVERWRITE", open: false,
    }},
  });
  return new Promise((resolve, reject) => {
    const request = rawHttpRequest(endpoint, {
      method: "POST",
      headers: {
        Host: host,
        Authorization: "Bearer " + bearer,
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
        "Content-Length": Buffer.byteLength(body),
      },
      timeout: 5000,
    }, response => {
      response.resume();
      response.on("end", () => resolve(response.statusCode));
    });
    request.on("timeout", () => request.destroy(new Error("Host denial timed out")));
    request.on("error", reject);
    request.end(body);
  });
}

const { sdk: agentA, transport: transportA } = client("agent-a");
const { sdk: agentB, transport: transportB } = client("agent-b");
let connectedA = false;
let connectedB = false;
try {
  await agentA.connect(transportA);
  connectedA = true;
  await agentB.connect(transportB);
  connectedB = true;

  assert.equal(transportA.protocolVersion, "2025-06-18", "unexpected negotiated MCP version");
  assert.equal(transportB.protocolVersion, "2025-06-18");
  assert.equal(agentA.getServerVersion()?.name, "aquilum");

  const listed = await agentA.listTools();
  assert.ok(listed.tools?.some(t => t.name === "read_note"), "read_note not advertised");
  assert.ok(listed.tools?.some(t => t.name === "update_note"), "update_note not advertised");

  const args = { note: "Conformance.md", workspace, includeRevision: true };
  const a = toolJson(await agentA.callTool({ name: "read_note", arguments: args }));
  const b = toolJson(await agentB.callTool({ name: "read_note", arguments: args }));
  assert.equal(a.content, "# Conformance\n\nTemporary note.\n");
  assert.equal(b.content, a.content);
  assert.match(a.revision, /^[0-9a-f]{64}$/);
  assert.equal(b.revision, a.revision);

  // Denied HTTP writes cannot reach File Gate.
  const missingBearer = await deniedWrite();
  assert.equal(missingBearer.status, 401);
  const wrongBearer = await deniedWrite({ Authorization: "Bearer wrong-credential" });
  assert.equal(wrongBearer.status, 401);
  const badOrigin = await deniedWrite({
    Authorization: "Bearer " + bearer,
    Origin: "https://attacker.example",
  });
  assert.equal(badOrigin.status, 403);
  const wrongVersion = await deniedWrite({
    Authorization: "Bearer " + bearer,
    "MCP-Protocol-Version": "2026-07-28",
  });
  assert.equal(wrongVersion.status, 400);
  for (const host of [
    "attacker.example",
    "localhost.evil.example",
    "127.0.0.1.evil.example",
    "127.0.0.1@" + "attacker.example",
    "127.0.0.1:" + (Number(port) === 65535 ? 65534 : Number(port) + 1),
  ]) {
    assert.equal(await deniedHost(host), 403, "Host rebinding denial: " + host);
  }
  const stillOriginal = toolJson(await agentA.callTool({ name: "read_note", arguments: args }));
  assert.equal(stillOriginal.revision, a.revision, "denied request modified the document");
  assert.equal(stillOriginal.content, a.content);
  assert.ok(!stillOriginal.content.includes("HOST_REBINDING_OVERWRITE"));

  // Two independent official SDK clients race on one raw-byte revision.
  // Exactly one may change the note, and the loser must re-read.
  const edits = await Promise.all([
    agentA.callTool({ name: "update_note", arguments: {
      note: "Conformance.md", workspace, mode: "append",
      content: "AKAI_DIRECT_AGENT_A", expectedRevision: a.revision, open: false,
    }}),
    agentB.callTool({ name: "update_note", arguments: {
      note: "Conformance.md", workspace, mode: "append",
      content: "AKAI_DIRECT_AGENT_B", expectedRevision: b.revision, open: false,
    }}),
  ]);
  const winners = edits.filter(x => x.isError !== true);
  const losers = edits.filter(x => x.isError === true);
  assert.equal(winners.length, 1, "expected exactly one CAS commit");
  assert.equal(losers.length, 1, "expected exactly one CAS conflict");
  const committed = toolJson(winners[0]);
  assert.equal(committed.changed, true);
  assert.match(committed.revision, /^[0-9a-f]{64}$/);
  assert.notEqual(committed.revision, a.revision);
  assert.match(losers[0].content?.[0]?.text || "", /MCP_REVISION_CONFLICT/);

  const after = toolJson(await agentB.callTool({ name: "read_note", arguments: args }));
  assert.equal(after.revision, committed.revision);
  assert.notEqual(after.content.includes("AKAI_DIRECT_AGENT_A"), after.content.includes("AKAI_DIRECT_AGENT_B"));
  assert.ok(!after.content.includes("UNAUTHORIZED_OVERWRITE"));

  console.log("AKAI_DIRECT_SDK_RESULT initialize=PASS tools_list=PASS read_note=PASS");
  console.log("AKAI_DIRECT_SDK_RESULT auth_missing=401 auth_wrong=401 origin_wrong=403 version_unsupported=400");
  console.log("AKAI_DIRECT_SDK_RESULT host_rebinding_denied=5 leaked_writes=0");
  console.log("AKAI_DIRECT_SDK_RESULT concurrent_cas_winners=1 conflicts=1 stale_overwrites=0");
} finally {
  if (connectedB) await agentB.close().catch(() => {});
  if (connectedA) await agentA.close().catch(() => {});
}
