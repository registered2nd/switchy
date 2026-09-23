/**
 * Format a JSON string
 * @param value - raw JSON string
 * @returns the formatted JSON string (2-space indent)
 * @throws if the JSON is invalid
 */
export function formatJSON(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return "";
  }
  const parsed = JSON.parse(trimmed);
  return JSON.stringify(parsed, null, 2);
}

/**
 * Smart parse of an MCP JSON config
 * Accepts two shapes:
 * 1. Bare config object: { "command": "npx", "args": [...], ... }
 * 2. Wrapped in a key:  "server-name": { "command": "npx", ... }  or  { "server-name": {...} }
 *
 * @param jsonText - JSON string
 * @returns { id?: string, config: object, formattedConfig: string }
 * @throws if the JSON is invalid
 */
export function parseSmartMcpJson(jsonText: string): {
  id?: string;
  config: any;
  formattedConfig: string;
} {
  let trimmed = jsonText.trim();
  if (!trimmed) {
    return { config: {}, formattedConfig: "" };
  }

  // A key-value fragment ("key": {...}) is wrapped into a full object
  if (trimmed.startsWith('"') && !trimmed.startsWith("{")) {
    trimmed = `{${trimmed}}`;
  }

  const parsed = JSON.parse(trimmed);

  // A single-key object whose value is an object: extract the key and the config
  const keys = Object.keys(parsed);
  if (
    keys.length === 1 &&
    parsed[keys[0]] &&
    typeof parsed[keys[0]] === "object" &&
    !Array.isArray(parsed[keys[0]])
  ) {
    const id = keys[0];
    const config = parsed[id];
    return {
      id,
      config,
      formattedConfig: JSON.stringify(config, null, 2),
    };
  }

  // Otherwise use it as is
  return {
    config: parsed,
    formattedConfig: JSON.stringify(parsed, null, 2),
  };
}

/**
 * TOML formatting is disabled
 *
 * Reason: smol-toml parse/stringify drops every comment and the original layout.
 * TOML is mostly used for config files, where comments are documentation; losing them hurts users badly.
 *
 * Possible future options:
 * - @ltd/j-toml (keeps comments, but adds a dependency and a complex API)
 * - a lightweight formatter that only touches indentation/whitespace
 * - toml-eslint-parser plus a custom generator
 *
 * For now: rely on the existing TOML syntax validation (useCodexTomlValidation) and offer no formatting.
 */
