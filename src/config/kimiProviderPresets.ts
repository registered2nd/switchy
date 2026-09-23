/**
 * Kimi Code provider preset templates
 *
 * A Kimi provider record is `{ config, credentials }`:
 * - `config` is the full text of `~/.kimi-code/config.toml`
 * - `credentials` is `~/.kimi-code/credentials/kimi-code.json` (managed login) or null
 */
import { ProviderCategory } from "../types";
import type { PresetTheme } from "./claudeProviderPresets";

export type KimiProviderType = "openai" | "anthropic" | "kimi";

export interface KimiProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  // Separate "get an API key" link for third-party providers
  apiKeyUrl?: string;
  config: string; // Written to ~/.kimi-code/config.toml (TOML string)
  credentials: Record<string, any> | null; // Written to credentials/kimi-code.json
  isOfficial?: boolean; // Official preset
  category?: ProviderCategory; // Category
  isCustomTemplate?: boolean; // Custom template
  // Candidate endpoints (for endpoint management and speed tests)
  endpointCandidates?: string[];
  // Visual theme
  theme?: PresetTheme;
  // Icon
  icon?: string;
  iconColor?: string;
}

const TOML_BARE_KEY = /^[A-Za-z0-9_-]+$/;

/** Key for a TOML table header: bare keys as-is, anything else double-quoted */
export function quoteKimiTomlKey(key: string): string {
  return TOML_BARE_KEY.test(key) ? key : `"${key}"`;
}

/**
 * Build config.toml for a third-party provider
 *
 * `default_model` uses the `<providerId>/<model>` alias; the alias table points back to the provider.
 */
export function generateThirdPartyConfig(
  providerId: string,
  apiKey: string,
  baseUrl: string,
  model = "gpt-4o",
  type: KimiProviderType = "openai",
): string {
  const cleanProviderId =
    providerId
      .toLowerCase()
      .replace(/[^a-z0-9_-]/g, "-")
      .replace(/^-+|-+$/g, "") || "custom";
  const alias = `${cleanProviderId}/${model}`;
  const providerLines = [
    `[providers.${quoteKimiTomlKey(cleanProviderId)}]`,
    `type = "${type}"`,
    `api_key = "${apiKey}"`,
  ];
  if (baseUrl.trim()) {
    providerLines.push(`base_url = "${baseUrl.trim().replace(/\/+$/, "")}"`);
  }

  return `default_model = "${alias}"

${providerLines.join("\n")}

[models."${alias}"]
provider = "${cleanProviderId}"
model = "${model}"`;
}

/** Official (managed login) config.toml: the provider and model tables kimi-code 0.26 writes */
export const KIMI_OFFICIAL_CONFIG = `default_model = "kimi-code/kimi-for-coding"

[providers."managed:kimi-code"]
type = "kimi"
api_key = ""
base_url = "https://api.kimi.com/coding/v1"

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use" ]
display_name = "K2.8 Preview"
support_efforts = [ "low", "high", "max" ]
default_effort = "max"

[models."kimi-code/kimi-for-coding-highspeed"]
provider = "managed:kimi-code"
model = "kimi-for-coding-highspeed"
max_context_size = 262144
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use" ]
display_name = "K2.7 Code Highspeed"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use" ]
display_name = "K3"
support_efforts = [ "low", "high", "max" ]
default_effort = "high"

[models."kimi-code/k3-256k"]
provider = "managed:kimi-code"
model = "k3-256k"
max_context_size = 262144
capabilities = [ "thinking", "always_thinking", "image_in", "tool_use" ]
display_name = "K3-256k"
support_efforts = [ "low", "high", "max" ]
default_effort = "high"`;

export const kimiProviderPresets: KimiProviderPreset[] = [
  {
    name: "Kimi Code",
    websiteUrl: "https://www.kimi.com/code",
    isOfficial: true,
    category: "official",
    config: KIMI_OFFICIAL_CONFIG,
    credentials: null,
    theme: {
      icon: "kimi",
      backgroundColor: "#1F2937", // gray-800
      textColor: "#FFFFFF",
    },
    icon: "kimi",
    iconColor: "#6366F1",
  },
  {
    name: "OpenAI Compatible",
    websiteUrl: "https://moonshotai.github.io/kimi-code/",
    category: "third_party",
    config: generateThirdPartyConfig("custom", "", "", "gpt-4o", "openai"),
    credentials: null,
    icon: "openai",
    iconColor: "#00A67E",
  },
  {
    name: "Anthropic Compatible",
    websiteUrl: "https://moonshotai.github.io/kimi-code/",
    category: "third_party",
    config: generateThirdPartyConfig(
      "custom",
      "",
      "",
      "claude-sonnet-4-5",
      "anthropic",
    ),
    credentials: null,
    icon: "anthropic",
    iconColor: "#D97757",
  },
];
