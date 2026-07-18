import assert from "node:assert/strict";
import { access, copyFile, readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, mkdir, mkdtemp, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const scripts = path.join(root, "plugins", "nodestorm", "skills", "nodestorm", "scripts");

for (const [os, arch, asset] of [
  ["linux", "x64", "nodestorm-v0.9.0-linux-x64.tar.gz"],
  ["linux", "arm64", "nodestorm-v0.9.0-linux-arm64.tar.gz"],
  ["macos", "x64", "nodestorm-v0.9.0-macos-x64.zip"],
  ["macos", "arm64", "nodestorm-v0.9.0-macos-arm64.zip"],
]) {
  test(`POSIX setup dry-run maps ${os}/${arch}`, () => {
    const result = spawnSync(
      "bash",
      [path.join(scripts, "setup.sh"), "--dry-run", "--os", os, "--arch", arch],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, new RegExp(asset.replaceAll(".", "\\.")));
  });
}

test("Windows setup supports both Store architectures and has no direct download", async () => {
  const script = await readFile(path.join(scripts, "setup.ps1"), "utf8");
  assert.match(script, /ValidateSet\("x64",\s*"arm64"\)/);
  assert.match(script, /install\s+--id[\s\S]*--source\s+msstore/i);
  assert.match(script, /ms-windows-store:\/\/pdp\/\?ProductId=/);
  assert.match(script, /0\.9\.0/);
  assert.match(script, /Get-AppxPackage -Name \$Store\.identityName/);
  assert.match(script, /\$Store\.msixVersion/);
  assert.match(script, /Microsoft\\WindowsApps/);
  assert.doesNotMatch(script, /Invoke-WebRequest[\s\S]*\.exe|github\.com[\s\S]*windows/i);
});

test("setup scripts require explicit install and launch consent", async () => {
  const shell = await readFile(path.join(scripts, "setup.sh"), "utf8");
  const powershell = await readFile(path.join(scripts, "setup.ps1"), "utf8");
  assert.match(shell, /Install Nodestorm v\$VERSION/);
  assert.match(shell, /Launch Nodestorm now/);
  assert.match(powershell, /Install Nodestorm v\$Version/);
  assert.match(powershell, /Launch Nodestorm now/);
});

async function executable(file, body) {
  await writeFile(file, body);
  await chmod(file, 0o755);
}

async function linkCommands(bin, names) {
  for (const name of names) {
    const command = spawnSync("bash", ["-lc", `command -v ${name}`], { encoding: "utf8" }).stdout.trim();
    assert.ok(command, `test host is missing ${name}`);
    await symlink(command, path.join(bin, name));
  }
}

async function linuxFailureFixture({ checksumValid = true, ghExit = 0, missingLibrary = false } = {}) {
  const fixture = await mkdtemp(path.join(os.tmpdir(), "nodestorm-installer-"));
  const release = path.join(fixture, "release");
  const staging = path.join(fixture, "staging");
  const bin = path.join(fixture, "bin");
  await mkdir(release);
  await mkdir(staging);
  await mkdir(bin);
  const binary = path.join(staging, "nodestorm");
  await executable(binary, '#!/bin/bash\nif [[ "${1:-}" == "--version" ]]; then echo "nodestorm 0.9.0"; fi\n');
  for (const size of [128, 256, 512]) {
    const iconDir = path.join(staging, "icons", `${size}x${size}`);
    await mkdir(iconDir, { recursive: true });
    await copyFile(path.join(root, "assets", "icons", `nodestorm-${size}.png`), path.join(iconDir, "nodestorm.png"));
  }
  const asset = "nodestorm-v0.9.0-linux-x64.tar.gz";
  const tar = spawnSync("tar", ["-C", staging, "-czf", path.join(release, asset), "nodestorm", "icons"], { encoding: "utf8" });
  assert.equal(tar.status, 0, tar.stderr);
  const archive = await readFile(path.join(release, asset));
  const digest = checksumValid ? createHash("sha256").update(archive).digest("hex") : "0".repeat(64);
  await writeFile(path.join(release, "SHA256SUMS"), `${digest}  ${asset}\n`);

  await linkCommands(bin, ["dirname", "tr", "curl", "sha256sum", "tar", "gzip", "mktemp", "rm", "mkdir", "install", "chmod", "grep", "sleep"]);
  await executable(path.join(bin, "gh"), `#!/bin/bash\nexit ${ghExit}\n`);
  await executable(
    path.join(bin, "ldd"),
    missingLibrary
      ? '#!/bin/bash\necho "libwebkit2gtk-4.1.so.0 => not found"\n'
      : '#!/bin/bash\necho "libc.so.6 => /lib/libc.so.6"\n',
  );

  return {
    fixture,
    env: {
      ...process.env,
      PATH: bin,
      XDG_DATA_HOME: path.join(fixture, "data"),
      NODESTORM_SETUP_TESTING: "1",
      NODESTORM_RELEASE_BASE_URL: pathToFileURL(release).href,
      NODESTORM_DOWNLOAD_PROTOCOL: "=file",
      NODESTORM_READINESS_ATTEMPTS: "1",
    },
  };
}

test("Linux setup installs launcher and hicolor icons", async () => {
  const fixture = await linuxFailureFixture();
  const result = spawnSync(
    "/bin/bash",
    [path.join(scripts, "setup.sh"), "--os", "linux", "--arch", "x64", "--approve-install", "--skip-launch"],
    { env: fixture.env, encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  const data = fixture.env.XDG_DATA_HOME;
  assert.equal(await readFile(path.join(data, "applications", "nodestorm.desktop"), "utf8").then((s) => s.includes("Icon=nodestorm")), true);
  for (const size of [128, 256, 512]) {
    await access(path.join(data, "icons", "hicolor", `${size}x${size}`, "apps", "nodestorm.png"));
  }
});

function runLinuxFixture(env) {
  return spawnSync(
    "/bin/bash",
    [path.join(scripts, "setup.sh"), "--os", "linux", "--arch", "x64", "--approve-install", "--approve-launch"],
    { env, encoding: "utf8" },
  );
}

test("Linux setup executes checksum and provenance failure paths", async () => {
  const badChecksum = await linuxFailureFixture({ checksumValid: false });
  const checksumResult = runLinuxFixture(badChecksum.env);
  assert.notEqual(checksumResult.status, 0);
  assert.match(`${checksumResult.stdout}\n${checksumResult.stderr}`, /FAILED|checksum/i);

  const badAttestation = await linuxFailureFixture({ ghExit: 23 });
  const attestationResult = runLinuxFixture(badAttestation.env);
  assert.equal(attestationResult.status, 23);
});

test("Linux setup executes missing-library and readiness failure paths", async () => {
  const missingLibrary = await linuxFailureFixture({ missingLibrary: true });
  const libraryResult = runLinuxFixture(missingLibrary.env);
  assert.notEqual(libraryResult.status, 0);
  assert.match(libraryResult.stderr, /Missing Linux runtime libraries/);

  const noServer = await linuxFailureFixture();
  const readinessResult = runLinuxFixture(noServer.env);
  assert.notEqual(readinessResult.status, 0);
  assert.match(readinessResult.stderr, /MCP readiness timed out/);
});

test("macOS setup executes a signing failure path", async (t) => {
  const zipCommand = spawnSync("bash", ["-lc", "command -v zip"], { encoding: "utf8" }).stdout.trim();
  if (!zipCommand) return t.skip("zip is unavailable on this test host");
  const fixture = await mkdtemp(path.join(os.tmpdir(), "nodestorm-signing-"));
  const release = path.join(fixture, "release");
  const staging = path.join(fixture, "staging");
  const appBinaryDir = path.join(staging, "Nodestorm.app", "Contents", "MacOS");
  const bin = path.join(fixture, "bin");
  await mkdir(release);
  await mkdir(appBinaryDir, { recursive: true });
  await mkdir(bin);
  await executable(path.join(appBinaryDir, "nodestorm"), '#!/bin/bash\necho "nodestorm 0.9.0"\n');
  const asset = "nodestorm-v0.9.0-macos-x64.zip";
  const zip = spawnSync(zipCommand, ["-qry", path.join(release, asset), "Nodestorm.app"], { cwd: staging, encoding: "utf8" });
  assert.equal(zip.status, 0, zip.stderr);
  const archive = await readFile(path.join(release, asset));
  const digest = createHash("sha256").update(archive).digest("hex");
  await writeFile(path.join(release, "SHA256SUMS"), `${digest}  ${asset}\n`);
  await linkCommands(bin, ["dirname", "tr", "curl", "shasum", "unzip", "mktemp", "rm", "grep"]);
  await executable(path.join(bin, "codesign"), "#!/bin/bash\nexit 19\n");
  await executable(path.join(bin, "spctl"), "#!/bin/bash\nexit 0\n");
  await executable(path.join(bin, "open"), "#!/bin/bash\nexit 0\n");

  const result = spawnSync(
    "/bin/bash",
    [path.join(scripts, "setup.sh"), "--os", "macos", "--arch", "x64", "--approve-install", "--approve-launch"],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: bin,
        NODESTORM_SETUP_TESTING: "1",
        NODESTORM_RELEASE_BASE_URL: pathToFileURL(release).href,
        NODESTORM_DOWNLOAD_PROTOCOL: "=file",
      },
    },
  );
  assert.equal(result.status, 19);
});
