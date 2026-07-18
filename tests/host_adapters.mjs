import assert from "node:assert/strict";
import test from "node:test";

const pluginRoot = new URL("../plugins/nodestorm/", import.meta.url);

test("OpenCode preserves an explicit MCP override and deduplicates skill paths", async () => {
  const { default: nodestorm } = await import(new URL("opencode.js", pluginRoot));
  const hooks = await nodestorm({});
  const override = { type: "remote", url: "https://example.test/mcp" };
  const config = {
    mcp: { nodestorm: override },
    skills: { paths: ["/existing", "/existing"] },
  };

  await hooks.config(config);
  await hooks.config(config);

  assert.equal(config.mcp.nodestorm, override);
  assert.equal(config.skills.paths.filter((item) => item === "/existing").length, 2);
  assert.equal(config.skills.paths.filter((item) => item.endsWith("/plugins/nodestorm/skills")).length, 1);
});

test("OpenCode supplies the local remote MCP when no override exists", async () => {
  const { default: nodestorm } = await import(new URL("opencode.js", pluginRoot));
  const hooks = await nodestorm({});
  const config = {};

  await hooks.config(config);

  assert.deepEqual(config.mcp.nodestorm, {
    type: "remote",
    url: "http://127.0.0.1:4747/mcp",
    enabled: true,
    timeout: 600_000,
  });
  assert.equal(config.skills.paths.length, 1);
});

function piHarness() {
  const handlers = new Map();
  return {
    tools: [],
    handlers,
    registerTool(tool) {
      this.tools.push(tool);
    },
    on(event, handler) {
      handlers.set(event, handler);
    },
  };
}

test("Pi lazily proxies all tools with object args and a ten-minute timeout", async () => {
  const { createNodestormExtension, NODESTORM_TOOLS } = await import(new URL("pi.js", pluginRoot));
  const calls = [];
  let connections = 0;
  const client = {
    async callTool(request, _schema, options) {
      calls.push({ request, options });
      return { content: [{ type: "text", text: "proxied" }] };
    },
    async close() {},
  };
  const pi = piHarness();
  createNodestormExtension({
    async createClient() {
      connections += 1;
      return client;
    },
  })(pi);

  assert.equal(pi.tools.length, 1);
  assert.equal(pi.tools[0].name, "nodestorm");
  assert.deepEqual(pi.tools[0].parameters.properties.tool.enum, NODESTORM_TOOLS);
  const result = await pi.tools[0].execute("call-1", {
    tool: "get_state",
    args: { session: "design" },
  });
  await pi.tools[0].execute("call-2", { tool: "list_sessions", args: {} });

  assert.equal(connections, 1);
  assert.deepEqual(calls[0].request, { name: "get_state", arguments: { session: "design" } });
  assert.equal(calls[0].options.timeout, 600_000);
  assert.equal(result.content[0].text, "proxied");
});

test("Pi surfaces MCP failures and closes the lazy client at shutdown", async () => {
  const { createNodestormExtension } = await import(new URL("pi.js", pluginRoot));
  let closed = 0;
  const client = {
    async callTool() {
      throw new Error("MCP unavailable");
    },
    async close() {
      closed += 1;
    },
  };
  const pi = piHarness();
  createNodestormExtension({ createClient: async () => client })(pi);

  await assert.rejects(
    () => pi.tools[0].execute("call-1", { tool: "get_state", args: {} }),
    /MCP unavailable/,
  );
  await pi.handlers.get("session_shutdown")();
  assert.equal(closed, 1);
});

test("Pi turns MCP tool error results into failed proxy calls", async () => {
  const { createNodestormExtension } = await import(new URL("pi.js", pluginRoot));
  const pi = piHarness();
  createNodestormExtension({
    createClient: async () => ({
      async callTool() {
        return { isError: true, content: [{ type: "text", text: "invalid graph" }] };
      },
      async close() {},
    }),
  })(pi);

  await assert.rejects(
    () => pi.tools[0].execute("call-1", { tool: "propose_graph", args: {} }),
    /invalid graph/,
  );
});
