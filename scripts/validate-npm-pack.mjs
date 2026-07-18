import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const plugin = path.join(root, "plugins", "nodestorm");
const result = spawnSync("npm", ["pack", "--dry-run", "--json"], {
  cwd: plugin,
  encoding: "utf8",
});
if (result.status !== 0) throw new Error(result.stderr || result.stdout);
const report = JSON.parse(result.stdout)[0];
const files = report.files.map(({ path: file }) => file);

for (const required of [
  "package.json",
  ".mcp.json",
  ".claude-plugin/plugin.json",
  ".codex-plugin/plugin.json",
  "LICENSE-APACHE",
  "LICENSE-MIT",
  "opencode.js",
  "pi.js",
  "skills/nodestorm/SKILL.md",
  "skills/nodestorm/scripts/setup.sh",
  "skills/nodestorm/scripts/setup.ps1",
]) assert.ok(files.includes(required), `npm package is missing ${required}`);

for (const file of files) {
  assert.doesNotMatch(file, /(^|\/)(src|tests|packaging|target)(\/|$)/);
  assert.doesNotMatch(file, /secret|\.p12$|store-identity\.json$/i);
}

console.log(`npm pack contains ${files.length} publishable files and no source-only or secret files.`);
