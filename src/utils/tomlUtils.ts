import { parse as parseToml, stringify as stringifyToml } from "smol-toml";
import { normalizeTomlText } from "@/utils/textNormalization";
import { McpServerSpec } from "../types";

/**
 * Validate TOML and convert it to a JSON object
 * @param text TOML text
 * @returns error message (empty string means success)
 */
export const validateToml = (text: string): string => {
  if (!text.trim()) return "";
  try {
    const normalized = normalizeTomlText(text);
    const parsed = parseToml(normalized);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return "mustBeObject";
    }
    return "";
  } catch (e: any) {
    // Return the underlying error; the caller wraps it for i18n
    return e?.message || "parseError";
  }
};

/**
 * Convert a McpServerSpec object to a TOML string
 * Uses @iarna/toml stringify, which handles escaping and nested tables
 * Keeps every field (including extension fields such as timeout_ms)
 */
export const mcpServerToToml = (server: McpServerSpec): string => {
  // Copy every field first (keeps extension fields)
  const obj: any = { ...server };

  // Drop undefined fields for cleaner output
  for (const k of Object.keys(obj)) {
    if (obj[k] === undefined) delete obj[k];
  }

  // stringify adds a trailing newline; trim it for the text box
  return stringifyToml(obj).trim();
};

/**
 * Convert TOML text to a McpServerSpec object (one server config)
 * Accepted shapes:
 * 1. A bare server config (type, command, args etc.)
 * 2. [mcp_servers.<id>] (recommended; takes the first server)
 * 3. [mcp.servers.<id>] (wrong shape, parsed leniently; takes the first server)
 * @param tomlText TOML text
 * @returns McpServer object
 * @throws when parsing or conversion fails
 */
export const tomlToMcpServer = (tomlText: string): McpServerSpec => {
  if (!tomlText.trim()) {
    throw new Error("TOML content cannot be empty");
  }

  const parsed = parseToml(normalizeTomlText(tomlText));

  // Case 1: a bare server config (has type/command/url etc.)
  if (
    parsed.type ||
    parsed.command ||
    parsed.url ||
    parsed.args ||
    parsed.env
  ) {
    return normalizeServerConfig(parsed);
  }

  // Case 2: [mcp_servers.<id>] (recommended)
  if (parsed.mcp_servers && typeof parsed.mcp_servers === "object") {
    const serverIds = Object.keys(parsed.mcp_servers);
    if (serverIds.length > 0) {
      const firstServer = (parsed.mcp_servers as any)[serverIds[0]];
      return normalizeServerConfig(firstServer);
    }
  }

  // Case 3: [mcp.servers.<id>] wrong shape (parsed leniently)
  if (parsed.mcp && typeof parsed.mcp === "object") {
    const mcpObj = parsed.mcp as any;
    if (mcpObj.servers && typeof mcpObj.servers === "object") {
      const serverIds = Object.keys(mcpObj.servers);
      if (serverIds.length > 0) {
        const firstServer = mcpObj.servers[serverIds[0]];
        return normalizeServerConfig(firstServer);
      }
    }
  }

  throw new Error(
    "Unrecognized TOML format. Provide a single MCP server config or use [mcp_servers.<id>]",
  );
};

/**
 * Normalize a server config object into McpServer shape
 * Keeps every field (including extension fields such as timeout_ms)
 */
function normalizeServerConfig(config: any): McpServerSpec {
  if (!config || typeof config !== "object") {
    throw new Error("Server config must be an object");
  }

  const type = (config.type as string) || "stdio";

  // Known fields (excluded later)
  const knownFields = new Set<string>();

  if (type === "stdio") {
    if (!config.command || typeof config.command !== "string") {
      throw new Error("A stdio MCP server must have a command field");
    }

    const server: McpServerSpec = {
      type: "stdio",
      command: config.command,
    };
    knownFields.add("type");
    knownFields.add("command");

    // Optional fields
    if (config.args && Array.isArray(config.args)) {
      server.args = config.args.map((arg: any) => String(arg));
      knownFields.add("args");
    }
    if (config.env && typeof config.env === "object") {
      const env: Record<string, string> = {};
      for (const [k, v] of Object.entries(config.env)) {
        env[k] = String(v);
      }
      server.env = env;
      knownFields.add("env");
    }
    if (config.cwd && typeof config.cwd === "string") {
      server.cwd = config.cwd;
      knownFields.add("cwd");
    }

    // Keep every unknown field (extension fields such as timeout_ms)
    for (const key of Object.keys(config)) {
      if (!knownFields.has(key)) {
        server[key] = config[key];
      }
    }

    return server;
  } else if (type === "http" || type === "sse") {
    if (!config.url || typeof config.url !== "string") {
      throw new Error(`A ${type} MCP server must have a url field`);
    }

    const server: McpServerSpec = {
      type: type as "http" | "sse",
      url: config.url,
    };
    knownFields.add("type");
    knownFields.add("url");

    // Optional fields
    if (config.headers && typeof config.headers === "object") {
      const headers: Record<string, string> = {};
      for (const [k, v] of Object.entries(config.headers)) {
        headers[k] = String(v);
      }
      server.headers = headers;
      knownFields.add("headers");
    }

    // Keep every unknown field
    for (const key of Object.keys(config)) {
      if (!knownFields.has(key)) {
        server[key] = config[key];
      }
    }

    return server;
  } else {
    throw new Error(`Unsupported MCP server type: ${type}`);
  }
}

/**
 * Try to extract a sensible server ID/title from TOML
 * @param tomlText TOML text
 * @returns suggested ID, or an empty string on failure
 */
export const extractIdFromToml = (tomlText: string): string => {
  try {
    const parsed = parseToml(normalizeTomlText(tomlText));

    // Try to take the ID from [mcp_servers.<id>] or [mcp.servers.<id>]
    if (parsed.mcp_servers && typeof parsed.mcp_servers === "object") {
      const serverIds = Object.keys(parsed.mcp_servers);
      if (serverIds.length > 0) {
        return serverIds[0];
      }
    }

    if (parsed.mcp && typeof parsed.mcp === "object") {
      const mcpObj = parsed.mcp as any;
      if (mcpObj.servers && typeof mcpObj.servers === "object") {
        const serverIds = Object.keys(mcpObj.servers);
        if (serverIds.length > 0) {
          return serverIds[0];
        }
      }
    }

    // Try to infer it from command
    if (parsed.command && typeof parsed.command === "string") {
      const cmd = parsed.command.split(/[\\/]/).pop() || "";
      return cmd.replace(/\.(exe|bat|sh|js|py)$/i, "");
    }
  } catch {
    // Parse failed, return empty
  }

  return "";
};
