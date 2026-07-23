import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "..");
const plugin = path.join(root, "plugins", "nodestorm");
const expectedVersion = "1.0.0";
const expectedMsixVersion = "1.0.0.0";
const tools = [
  "propose_graph",
  "update_graph",
  "await_decisions",
  "get_state",
  "clear_session",
  "export_markdown",
  "list_sessions",
  "diff_sessions",
  "diff_record",
];

async function text(file) {
  return readFile(path.join(root, file), "utf8");
}

async function json(file) {
  return JSON.parse(await text(file));
}

test("all public package versions remain synchronized", async () => {
  const cargo = await text("Cargo.toml");
  const cargoLock = await text("Cargo.lock");
  const packageJson = await json("plugins/nodestorm/package.json");
  const claudeManifest = await json("plugins/nodestorm/.claude-plugin/plugin.json");
  const codexManifest = await json("plugins/nodestorm/.codex-plugin/plugin.json");
  const version = (await text("plugins/nodestorm/VERSION")).trim();
  const windowsIdentity = await json("packaging/windows/store-identity.example.json");

  assert.match(cargo, new RegExp(`^version = "${expectedVersion.replaceAll(".", "\\.")}"$`, "m"));
  assert.match(cargoLock, new RegExp(`name = "nodestorm"\\nversion = "${expectedVersion.replaceAll(".", "\\.")}"`));
  assert.equal(packageJson.version, expectedVersion);
  assert.equal(claudeManifest.version, expectedVersion);
  assert.equal(codexManifest.version, expectedVersion);
  assert.equal(version, expectedVersion);
  assert.equal(windowsIdentity.msixVersion, expectedMsixVersion);
});

test("canonical package contains every host adapter and one shared skill", async () => {
  const packageJson = await json("plugins/nodestorm/package.json");
  const claudeCatalog = await json(".claude-plugin/marketplace.json");
  const codexCatalog = await json(".agents/plugins/marketplace.json");
  const skill = await text("plugins/nodestorm/skills/nodestorm/SKILL.md");
  const openai = await text("plugins/nodestorm/agents/openai.yaml");
  const mcp = await json("plugins/nodestorm/.mcp.json");

  assert.equal(packageJson.name, "nodestorm");
  assert.equal(packageJson.exports["."], "./opencode.js");
  assert.deepEqual(packageJson.pi.extensions, ["./pi.js"]);
  assert.deepEqual(packageJson.pi.skills, ["./skills"]);
  assert.equal(claudeCatalog.plugins[0].source, "./plugins/nodestorm");
  assert.equal(codexCatalog.plugins[0].source.path, "./plugins/nodestorm");
  assert.equal(mcp.mcpServers.nodestorm.url, "http://127.0.0.1:4747/mcp");
  assert.equal(mcp.mcpServers.nodestorm.timeout, 600_000);
  assert.equal(mcp.mcpServers.nodestorm.tool_timeout_sec, 600);
  assert.match(skill, /^---\nname: nodestorm\n/m);
  assert.match(openai, /http:\/\/127\.0\.0\.1:4747\/mcp/);
  for (const tool of tools) assert.match(skill, new RegExp(`\\b${tool}\\b`));
});

test("package files and installers reference pinned trusted distribution", async () => {
  const packageJson = await json("plugins/nodestorm/package.json");
  const posixInstaller = await text("plugins/nodestorm/skills/nodestorm/scripts/setup.sh");
  const windowsInstaller = await text("plugins/nodestorm/skills/nodestorm/scripts/setup.ps1");
  const releaseWorkflow = await text(".github/workflows/release-build.yml");
  const publishWorkflow = await text(".github/workflows/release-publish.yml");

  assert.ok(packageJson.files.includes("skills/nodestorm/scripts/setup.sh"));
  assert.ok(packageJson.files.includes("skills/nodestorm/scripts/setup.ps1"));
  assert.match(posixInstaller, /SHA256SUMS/);
  assert.match(posixInstaller, /gh attestation verify/);
  assert.match(posixInstaller, /codesign/);
  assert.match(posixInstaller, /spctl/);
  assert.doesNotMatch(posixInstaller, /^\s*sudo\b|\blatest\b/m);
  assert.match(windowsInstaller, /--source\s+msstore/);
  assert.doesNotMatch(windowsInstaller, /github\.com.*windows|\.exe\s*$/m);
  assert.match(releaseWorkflow, /--draft(?:=|\s+)true/);
  assert.match(publishWorkflow, /npm publish/);
});
