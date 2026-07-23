import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "..");
const binary = process.env.NODESTORM_BINARY ?? path.join(root, "target", "debug", "nodestorm");

async function ephemeralPort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => server.listen(0, "127.0.0.1", resolve).once("error", reject));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

function piHarness() {
  const handlers = new Map();
  return {
    tool: undefined,
    handlers,
    registerTool(tool) { this.tool = tool; },
    on(event, handler) { handlers.set(event, handler); },
  };
}

test("Pi proxy calls the real headless Nodestorm MCP server", { timeout: 30_000 }, async (t) => {
  const port = await ephemeralPort();
  const state = await mkdtemp(path.join(os.tmpdir(), "nodestorm-pi-"));
  const server = spawn(binary, [
    "--headless",
    "--port", String(port),
    "--sessions-dir", path.join(state, "sessions"),
    "--prefs", path.join(state, "preferences.json"),
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let stderr = "";
  server.stderr.on("data", (chunk) => { stderr += chunk; });
  t.after(async () => {
    if (server.exitCode === null) server.kill("SIGINT");
    await Promise.race([
      new Promise((resolve) => server.once("exit", resolve)),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
  });

  const { createNodestormExtension } = await import("../plugins/nodestorm/pi.js");
  const pi = piHarness();
  createNodestormExtension({ url: `http://127.0.0.1:${port}/mcp` })(pi);

  let result;
  let lastError;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      result = await pi.tool.execute("roundtrip", { tool: "list_sessions", args: {} });
      break;
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }

  assert.ok(result, `Pi never connected to headless Nodestorm: ${lastError}\n${stderr}`);
  assert.match(result.content[0].text, /active|sessions/i);
  await pi.handlers.get("session_shutdown")();
});
