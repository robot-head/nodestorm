export const NODESTORM_TOOLS = Object.freeze([
  "propose_graph",
  "update_graph",
  "await_decisions",
  "get_state",
  "clear_session",
  "export_markdown",
  "list_sessions",
  "diff_sessions",
  "diff_record",
]);

const MCP_URL = "http://127.0.0.1:4747/mcp";
const CALL_TIMEOUT_MS = 600_000;

async function defaultCreateClient({ url }) {
  const [{ Client }, { StreamableHTTPClientTransport }] = await Promise.all([
    import("@modelcontextprotocol/sdk/client/index.js"),
    import("@modelcontextprotocol/sdk/client/streamableHttp.js"),
  ]);
  const client = new Client({ name: "nodestorm-pi", version: "0.9.0" });
  await client.connect(new StreamableHTTPClientTransport(new URL(url)));
  return client;
}

export function createNodestormExtension({ url = MCP_URL, createClient = defaultCreateClient } = {}) {
  return function nodestormExtension(pi) {
    let clientPromise;

    async function client() {
      clientPromise ??= Promise.resolve(createClient({ url })).catch((error) => {
        clientPromise = undefined;
        throw error;
      });
      return clientPromise;
    }

    pi.registerTool({
      name: "nodestorm",
      label: "Nodestorm",
      description: "Call one of the tools exposed by the local Nodestorm MCP server.",
      parameters: {
        type: "object",
        properties: {
          tool: {
            type: "string",
            enum: NODESTORM_TOOLS,
            description: "Logical Nodestorm tool name.",
          },
          args: {
            type: "object",
            additionalProperties: true,
            description: "Object-valued arguments passed to the selected tool.",
          },
        },
        required: ["tool"],
        additionalProperties: false,
      },
      async execute(_toolCallId, params, signal) {
        const connected = await client();
        const result = await connected.callTool(
          { name: params.tool, arguments: params.args ?? {} },
          undefined,
          { timeout: CALL_TIMEOUT_MS, signal },
        );
        if (result.isError) {
          const message = (result.content ?? [])
            .filter((item) => item.type === "text")
            .map((item) => item.text)
            .join("\n") || "unknown MCP tool error";
          throw new Error(`Nodestorm ${params.tool} failed: ${message}`);
        }
        return {
          content: result.content ?? [{ type: "text", text: JSON.stringify(result) }],
          details: { tool: params.tool },
        };
      },
    });

    pi.on("session_shutdown", async () => {
      if (!clientPromise) return;
      const connected = await clientPromise;
      clientPromise = undefined;
      await connected.close();
    });
  };
}

export default createNodestormExtension();
