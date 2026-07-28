import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { mismatchedTag, releaseVersion, root, versionPattern } from "../scripts/release-version.mjs";

const scripts = path.join(root, "plugins", "nodestorm", "skills", "nodestorm", "scripts");

test("release validation hard-fails missing Partner Center identity or a wrong tag", async () => {
  let identityExists = true;
  try {
    await access(path.join(root, "packaging", "windows", "store-identity.json"));
  } catch {
    identityExists = false;
  }
  // The tag must be *wrong* for the release version — the point is to prove
  // validation rejects a mismatch. Derived, so it stays wrong across bumps.
  const args = ["scripts/validate-release.mjs", "--release", "--tag", mismatchedTag(await releaseVersion())];
  const result = spawnSync("node", args, { cwd: root, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.match(`${result.stdout}\n${result.stderr}`, identityExists ? /tag .* does not match/ : /Partner Center identity is missing/);
});

test("npm is published before the GitHub release becomes public", async () => {
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-publish.yml"), "utf8");
  const npmPublish = workflow.indexOf("npm publish --provenance --access public");
  const githubPublish = workflow.indexOf("gh release edit \"v$VERSION\" --draft=false");

  assert.notEqual(npmPublish, -1);
  assert.notEqual(githubPublish, -1);
  assert.ok(npmPublish < githubPublish, "npm must be published before the GitHub draft is made public");
});

test("no workflow, script, or test pins the release version literally", async () => {
  // The bump used to mean editing ~25 files, and the wrong-tag fixture in this
  // very file silently turned correct at the next version. Everything now
  // derives from plugins/nodestorm/VERSION, so nothing may name it outright.
  const literal = new RegExp(versionPattern(await releaseVersion()));
  for (const file of [
    ".github/workflows/release-build.yml",
    ".github/workflows/release-publish.yml",
    "scripts/validate-release.mjs",
    "scripts/configure-store.mjs",
    "scripts/submit-store.mjs",
    "tests/installers.mjs",
    "tests/plugin_contract.mjs",
    "tests/release_gates.mjs",
    "tests/windows_installer.ps1",
  ]) {
    const contents = await readFile(path.join(root, file), "utf8");
    assert.doesNotMatch(contents, literal, `${file} hardcodes the release version`);
  }
});

test("Store submission is tag-gated and reads its credentials from secrets", async () => {
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-build.yml"), "utf8");
  const job = workflow.slice(workflow.indexOf("\n  store-submit:"), workflow.indexOf("\n  draft-release:"));

  // A manual dispatch rebuilds artifacts; it must never touch the live listing.
  assert.match(job, /if: github\.ref_type == 'tag'/);
  assert.match(job, /needs: \[validate, windows-bundle, store-assets\]/);
  for (const secret of ["MSSTORE_TENANT_ID", "MSSTORE_CLIENT_ID", "MSSTORE_CLIENT_SECRET"]) {
    assert.match(job, new RegExp(`${secret}: \\$\\{\\{ secrets\\.${secret} \\}\\}`));
  }
  // The API takes a zip, not the bundle itself.
  assert.match(job, /zip -j .*\.zip.*\.msixbundle/);

  const script = await readFile(path.join(root, "scripts", "submit-store.mjs"), "utf8");
  assert.match(script, /x-ms-blob-type/, "the SAS upload needs an explicit block-blob header");
  assert.match(script, /PendingDelete/, "superseded packages must be retired");
  assert.match(script, /PendingUpload/, "the new package must be marked for upload");
  assert.match(script, /CommitFailed/, "a rejected commit must fail the job");
});

test("the Store listing is refreshed from the changelog and this build's artwork", async () => {
  const script = await readFile(path.join(root, "scripts", "submit-store.mjs"), "utf8");

  // A submission clone carries the previous listing; leaving it untouched is
  // how a release ships the release before it.
  assert.match(script, /baseListing\.releaseNotes = releaseNotes/, "release notes must come from the changelog");
  assert.match(script, /changelogSection/);
  // The same images array holds the hand-uploaded store logos. Retiring those
  // strips artwork nothing in this pipeline can regenerate.
  assert.match(
    script,
    /image\.imageType === "Screenshot" \? \{ \.\.\.image, fileStatus: "PendingDelete" \} : image/,
    "only superseded screenshots may be retired",
  );
  assert.match(script, /imageType: "Screenshot"/);
  assert.match(script, /videoFileName/, "the trailer must be attached to the listing");

  // Assets are checked before the first API call: a submission referencing a
  // file the zip lacks fails at commit, leaving a pending submission behind.
  const assetCheck = script.indexOf("listing asset missing from");
  const firstApiCall = script.indexOf("await api(");
  assert.notEqual(assetCheck, -1);
  assert.ok(assetCheck < firstApiCall, "listing assets must be verified before any API call");

  const listing = JSON.parse(
    await readFile(path.join(root, "packaging", "windows", "store-listing.json"), "utf8"),
  );
  const verifier = await readFile(path.join(root, "scripts", "verify-windows.ps1"), "utf8");
  assert.ok(listing.screenshots.length <= 10, "the Store accepts at most 10 desktop screenshots");
  for (const shot of listing.screenshots) {
    assert.ok(shot.caption.length <= 200, `caption for ${shot.file} exceeds 200 characters`);
    // Every listed screenshot must be one the verifier actually captures, and
    // not one of its narrow-window shots (below the Store's 1366px floor).
    assert.ok(verifier.includes(`'${shot.file}'`), `${shot.file} is not captured by verify-windows.ps1`);
    assert.doesNotMatch(shot.file, /narrow/, "narrow-window captures are below the Store's 1366x768 floor");
  }
});

test("listing artwork is regenerated from the shipped build, and its failure blocks the release", async () => {
  const workflow = await readFile(path.join(root, ".github", "workflows", "release-build.yml"), "utf8");
  const job = workflow.slice(workflow.indexOf("\n  store-assets:"), workflow.indexOf("\n  # Push the built bundle"));

  // Blocking is the point: a listing that could not be regenerated must stop
  // the release rather than quietly ship the previous version's screenshots.
  assert.doesNotMatch(job, /continue-on-error/);
  // Hosted runners boot below the Store's screenshot floor.
  assert.match(job, /Set-DisplayResolution 1920 1080/);
  assert.match(job, /verify-windows\.ps1 -NoBuild -WindowSize 1600x1000/);
  assert.match(job, /record-demo\.ps1 -NoBuild -Publish/);
  // The Store rejects anything but exactly 1920x1080 for a trailer and its thumbnail.
  assert.match(job, /pad=1920:1080/);
  assert.match(job, /-c:a aac/, "a trailer with no audio stream risks certification");
  assert.match(job, /at least 1366x768/, "undersized screenshots must fail before upload");

  const draft = workflow.slice(workflow.indexOf("\n  draft-release:"));
  assert.match(draft, /needs: \[validate, linux, macos, windows-bundle, store-assets\]/);
  assert.match(draft, /node scripts\/release-notes\.mjs > notes\.md/);
  assert.match(draft, /--notes-file notes\.md/, "the GitHub release must publish the changelog too");
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

test("Store submission skips cleanly without credentials but rejects a partial set", async () => {
  const submit = path.join(root, "scripts", "submit-store.mjs");
  const run = (env) =>
    spawnSync(process.execPath, [submit, "nodestorm-windows.zip"], {
      encoding: "utf8",
      // Strip any real credentials so a developer's shell cannot change the result.
      env: {
        ...Object.fromEntries(
          Object.entries(process.env).filter(([name]) => !name.startsWith("MSSTORE_")),
        ),
        ...env,
      },
    });

  // No credentials: a legitimate state. The release build must stay green and
  // the operator must be told how to upload by hand.
  const skipped = run({});
  assert.equal(skipped.status, 0, skipped.stderr);
  assert.match(skipped.stdout, /Store submission skipped/);
  assert.match(skipped.stdout, /Partner Center/);
  // The manual path must reach the whole place the API path does. Instructions
  // that mention only the package ship the stale listing this pipeline exists
  // to prevent, and nothing downstream would catch it.
  assert.match(skipped.stdout, /store-assets/, "the fallback must refresh the screenshots");
  assert.match(skipped.stdout, /trailer\.mp4/, "the fallback must refresh the trailer");
  assert.match(skipped.stdout, /Release notes:/, "the fallback must hand over the release notes");
  assert.match(skipped.stderr, /^::warning title=/m, "must annotate the workflow run");

  // A partial set means someone believes submission is wired up. Skipping there
  // would silently not ship a release, so it has to fail.
  for (const partial of [
    { MSSTORE_TENANT_ID: "t" },
    { MSSTORE_TENANT_ID: "t", MSSTORE_CLIENT_ID: "c" },
  ]) {
    const result = run(partial);
    assert.notEqual(result.status, 0, `partial credentials must fail: ${Object.keys(partial)}`);
    assert.match(result.stderr, /incomplete Store credentials/);
  }
});
