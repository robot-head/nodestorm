import { fileURLToPath } from "node:url";

const MCP_URL = "http://127.0.0.1:4747/mcp";
const CALL_TIMEOUT_MS = 600_000;
const SKILLS_PATH = fileURLToPath(new URL("./skills", import.meta.url));

export default async function nodestormPlugin() {
  return {
    async config(config) {
      config.mcp ??= {};
      config.skills ??= {};
      config.skills.paths ??= [];

      if (!("nodestorm" in config.mcp)) {
        config.mcp.nodestorm = {
          type: "remote",
          url: MCP_URL,
          enabled: true,
          timeout: CALL_TIMEOUT_MS,
        };
      }

      if (!config.skills.paths.includes(SKILLS_PATH)) {
        config.skills.paths.push(SKILLS_PATH);
      }
    },
  };
}
