import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "..");
const scripts = path.join(root, "plugins", "nodestorm", "skills", "nodestorm", "scripts");

test("release validation hard-fails missing Partner Center identity or a wrong tag", async () => {
  let identityExists = true;
  try {
    await access(path.join(root, "packaging", "windows", "store-identity.json"));
  } catch {
    identityExists = false;
  }
  const args = identityExists
    ? ["scripts/validate-release.mjs", "--release", "--tag", "v0.9.1"]
    : ["scripts/validate-release.mjs", "--release", "--tag", "v0.9.0"];
  const result = spawnSync("node", args, { cwd: root, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(`${result.stdout}\n${result.stderr}`, identityExists ? /tag .* does not match/ : /Partner Center identity is missing/);
});

test("npm is published before the GitHub release becomes public", async () => {
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-publish.yml"), "utf8");
  const npmPublish = workflow.indexOf("npm publish --provenance --access public");
  const githubPublish = workflow.indexOf("gh release edit v0.9.0 --draft=false");

  assert.notEqual(npmPublish, -1);
  assert.notEqual(githubPublish, -1);
  assert.ok(npmPublish < githubPublish, "npm must be published before the GitHub draft is made public");
});

test("POSIX setup contains executable abort gates for every trust boundary", async () => {
  const script = await readFile(path.join(scripts, "setup.sh"), "utf8");
  for (const pattern of [
    /sha256sum --check/,
    /gh attestation verify/,
    /codesign --verify --deep --strict/,
    /spctl --assess --type execute/,
    /grep -q "not found"/,
    /Port 4747 is already in use/,
    /MCP readiness timed out/,
  ]) assert.match(script, pattern);

  const unsupported = spawnSync(
    "bash",
    [path.join(scripts, "setup.sh"), "--dry-run", "--os", "linux", "--arch", "riscv64"],
    { encoding: "utf8" },
  );
  assert.notEqual(unsupported.status, 0);
  assert.match(unsupported.stderr, /Unsupported target/);
});

test("Windows setup aborts unavailable Store, version, port, and readiness paths", async () => {
  const script = await readFile(path.join(scripts, "setup.ps1"), "utf8");
  for (const pattern of [
    /Store listing is unavailable/,
    /version does not match/,
    /Port 4747 is already in use/,
    /execution alias did not become available/,
    /MCP readiness timed out/,
  ]) assert.match(script, pattern);
  assert.doesNotMatch(script, /releases\/download|https?:\/\/[^\s"']+\.msix(?:bundle)?/i);
});

test("Windows package assets use the redesigned square icon without distortion", async () => {
  const script = await readFile(path.join(root, "packaging", "windows", "prepare-layout.ps1"), "utf8");

  assert.match(script, /assets[\\\/]icons[\\\/]nodestorm-1024\.png/i);
  assert.doesNotMatch(script, /docs[\\\/]demo[\\\/]poster\.png/i);
  assert.match(script, /Wide310x150Logo\.png/);
  assert.match(script, /\$x\s*=\s*\(\$asset\.Width\s*-\s*\$side\)\s*\/\s*2/i);
});

test("macOS app bundle packages the redesigned icon", async () => {
  const plist = await readFile(path.join(root, "packaging", "macos", "Info.plist"), "utf8");
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-build.yml"), "utf8");
  const macosWorkflow = workflow.slice(workflow.indexOf("\n  macos:"), workflow.indexOf("\n  windows:"));

  assert.match(plist, /<key>CFBundleIconFile<\/key><string>Nodestorm\.icns<\/string>/);
  for (const command of [
    'cp assets/icons/nodestorm-16.png "$ICONSET/icon_16x16.png"',
    'cp assets/icons/nodestorm-32.png "$ICONSET/icon_16x16@2x.png"',
    'cp assets/icons/nodestorm-32.png "$ICONSET/icon_32x32.png"',
    'cp assets/icons/nodestorm-64.png "$ICONSET/icon_32x32@2x.png"',
    'cp assets/icons/nodestorm-128.png "$ICONSET/icon_128x128.png"',
    'cp assets/icons/nodestorm-256.png "$ICONSET/icon_128x128@2x.png"',
    'cp assets/icons/nodestorm-256.png "$ICONSET/icon_256x256.png"',
    'cp assets/icons/nodestorm-512.png "$ICONSET/icon_256x256@2x.png"',
    'cp assets/icons/nodestorm-512.png "$ICONSET/icon_512x512.png"',
    'cp assets/icons/nodestorm-1024.png "$ICONSET/icon_512x512@2x.png"',
  ]) assert.ok(macosWorkflow.includes(command), `missing macOS icon mapping: ${command}`);

  const iconGeneration = macosWorkflow.indexOf('iconutil -c icns -o "$APP/Contents/Resources/Nodestorm.icns" "$ICONSET"');
  const iconCheck = macosWorkflow.indexOf('test -s "$APP/Contents/Resources/Nodestorm.icns"');
  const firstCodesign = macosWorkflow.indexOf("codesign");
  assert.notEqual(iconGeneration, -1);
  assert.notEqual(iconCheck, -1);
  assert.notEqual(firstCodesign, -1);
  assert.ok(iconGeneration < iconCheck && iconCheck < firstCodesign, "macOS icon must be generated and checked before codesign");
});

test("Linux release packages and installs launcher artwork", async () => {
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-build.yml"), "utf8");
  const linuxWorkflow = workflow.slice(workflow.indexOf("\n  linux:"), workflow.indexOf("\n  macos:"));
  const script = await readFile(path.join(scripts, "setup.sh"), "utf8");

  assert.match(linuxWorkflow, /mkdir -p dist\/icons\/\{48x48,128x128,256x256,512x512\}/);
  for (const size of [48, 128, 256, 512]) {
    assert.ok(
      linuxWorkflow.includes(`cp assets/icons/nodestorm-${size}.png dist/icons/${size}x${size}/nodestorm.png`),
      `missing Linux ${size}px icon mapping`,
    );
  }
  assert.match(linuxWorkflow, /tar -C dist .* nodestorm icons/);
  assert.match(script, /for size in 48 128 256 512/);
  assert.match(script, /icons\/hicolor\/\$\{size\}x\$\{size\}\/apps/);
  assert.match(script, /Icon=nodestorm/);
});
