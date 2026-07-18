import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const pluginRoot = path.join(root, "plugins", "nodestorm");
const expected = "0.9.0";
const msixExpected = "0.9.0.0";
const releaseMode = process.argv.includes("--release");
const tagIndex = process.argv.indexOf("--tag");

async function text(relative) {
  return readFile(path.join(root, relative), "utf8");
}

async function json(relative) {
  return JSON.parse(await text(relative));
}

const packageJson = await json("plugins/nodestorm/package.json");
const claude = await json("plugins/nodestorm/.claude-plugin/plugin.json");
const codex = await json("plugins/nodestorm/.codex-plugin/plugin.json");
const mcp = await json("plugins/nodestorm/.mcp.json");
const cargo = await text("Cargo.toml");
const cargoLock = await text("Cargo.lock");
const version = (await text("plugins/nodestorm/VERSION")).trim();
const plist = await text("packaging/macos/Info.plist");
const skill = await text("plugins/nodestorm/skills/nodestorm/SKILL.md");
const skillAgent = await text("plugins/nodestorm/skills/nodestorm/agents/openai.yaml");
const rootAgent = await text("plugins/nodestorm/agents/openai.yaml");
const piAdapter = await text("plugins/nodestorm/pi.js");
const windowsSetup = await text("plugins/nodestorm/skills/nodestorm/scripts/setup.ps1");

assert.equal(version, expected);
assert.equal(packageJson.version, expected);
assert.equal(claude.version, expected);
assert.equal(codex.version, expected);
assert.match(cargo, /^version = "0\.9\.0"$/m);
assert.match(cargoLock, /name = "nodestorm"\nversion = "0\.9\.0"/);
assert.match(plist, /<key>CFBundleShortVersionString<\/key><string>0\.9\.0<\/string>/);
assert.match(piAdapter, /version: "0\.9\.0"/);
assert.match(windowsSetup, /\$Version = "0\.9\.0"/);
assert.equal(mcp.mcpServers.nodestorm.timeout, 600_000);
assert.equal(mcp.mcpServers.nodestorm.tool_timeout_sec, 600);
assert.equal(mcp.mcpServers.nodestorm.url, "http://127.0.0.1:4747/mcp");
assert.match(skill, /^---\nname: nodestorm\n/);
assert.match(skillAgent, /value: "nodestorm"[\s\S]*transport: "streamable_http"/);
assert.match(rootAgent, /http:\/\/127\.0\.0\.1:4747\/mcp/);

for (const tool of [
  "propose_graph", "update_graph", "await_decisions", "get_state",
  "clear_session", "export_markdown", "list_sessions", "diff_sessions",
]) {
  assert.match(skill, new RegExp(`\\b${tool}\\b`));
  assert.match(piAdapter, new RegExp(`"${tool}"`));
}

for (const file of packageJson.files) await access(path.join(pluginRoot, file));
await assert.rejects(access(path.join(root, "skills", "nodestorm", "SKILL.md")));

if (tagIndex !== -1) {
  const tag = process.argv[tagIndex + 1];
  assert.equal(tag, `v${expected}`, `tag ${tag} does not match package version ${expected}`);
}

const identityFile = releaseMode
  ? "packaging/windows/store-identity.json"
  : "packaging/windows/store-identity.example.json";
let identity;
try {
  identity = await json(identityFile);
} catch (error) {
  if (releaseMode) {
    throw new Error("Partner Center identity is missing: commit packaging/windows/store-identity.json before release.", { cause: error });
  }
  throw error;
}
assert.equal(identity.msixVersion, msixExpected);
for (const field of ["identityName", "publisher", "publisherDisplayName", "productId", "applicationId", "executionAlias"]) {
  assert.ok(identity[field], `missing Store identity field ${field}`);
  if (releaseMode) assert.doesNotMatch(identity[field], /^REPLACE_/, `unreserved Store identity field ${field}`);
}

if (releaseMode) {
  const storeSetup = await json("plugins/nodestorm/skills/nodestorm/scripts/store.json");
  assert.equal(storeSetup.identityName, identity.identityName, "plugin Store identity is stale; run scripts/configure-store.mjs");
  assert.equal(storeSetup.publisher, identity.publisher, "plugin Store publisher is stale; run scripts/configure-store.mjs");
  assert.equal(storeSetup.productId, identity.productId, "plugin Store Product ID is stale; run scripts/configure-store.mjs");
  assert.equal(storeSetup.executionAlias, identity.executionAlias, "plugin Store execution alias is stale; run scripts/configure-store.mjs");
  assert.equal(storeSetup.msixVersion, identity.msixVersion, "plugin Store MSIX version is stale; run scripts/configure-store.mjs");
  assert.equal(storeSetup.version, expected);
}

console.log(`Validated Nodestorm v${expected}${releaseMode ? " release gates" : " package contracts"}.`);
