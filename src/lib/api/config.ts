// Config API
import { invoke } from "@tauri-apps/api/core";

export type AppType =
  | "claude"
  | "codex"
  | "gemini"
  | "kimi"
  | "omo"
  | "omo_slim";

/**
 * Get the Claude common config snippet (deprecated, use getCommonConfigSnippet)
 * @returns the common config snippet (JSON string), or null if none
 * @deprecated use getCommonConfigSnippet('claude') instead
 */
export async function getClaudeCommonConfigSnippet(): Promise<string | null> {
  return invoke<string | null>("get_claude_common_config_snippet");
}

/**
 * Set the Claude common config snippet (deprecated, use setCommonConfigSnippet)
 * @param snippet - common config snippet (JSON string)
 * @throws if the JSON is invalid
 * @deprecated use setCommonConfigSnippet('claude', snippet) instead
 */
export async function setClaudeCommonConfigSnippet(
  snippet: string,
): Promise<void> {
  return invoke("set_claude_common_config_snippet", { snippet });
}

/**
 * Get the common config snippet (shared interface)
 * @param appType - app type (claude/codex/gemini)
 * @returns the common config snippet (raw string), or null if none
 */
export async function getCommonConfigSnippet(
  appType: AppType,
): Promise<string | null> {
  return invoke<string | null>("get_common_config_snippet", { appType });
}

/**
 * Set the common config snippet (shared interface)
 * @param appType - app type (claude/codex/gemini)
 * @param snippet - common config snippet (raw string)
 * @throws if the format is invalid (Claude/Gemini validate JSON; Codex is not validated yet)
 */
export async function setCommonConfigSnippet(
  appType: AppType,
  snippet: string,
): Promise<void> {
  return invoke("set_common_config_snippet", { appType, snippet });
}

/**
 * Extract the common config snippet
 *
 * Reads the active provider's config by default; with `options.settingsConfig`, extracts from the current editor content.
 * Leaves out provider-specific fields (API key, model settings, endpoints etc.) and returns a reusable common config snippet.
 *
 * @param appType - app type (claude/codex/gemini)
 * @param options - optional: extraction source
 * @returns the extracted common config snippet (JSON/TOML string)
 */
export type ExtractCommonConfigSnippetOptions = {
  settingsConfig?: string;
};

export async function extractCommonConfigSnippet(
  appType: Exclude<AppType, "omo">,
  options?: ExtractCommonConfigSnippetOptions,
): Promise<string> {
  const args: Record<string, unknown> = { appType };
  const settingsConfig = options?.settingsConfig;

  if (typeof settingsConfig === "string" && settingsConfig.trim()) {
    args.settingsConfig = settingsConfig;
  }

  return invoke<string>("extract_common_config_snippet", args);
}
