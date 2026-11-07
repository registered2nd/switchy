/**
 * Codex provider preset templates
 */
import { ProviderCategory } from "../types";
import type { PresetTheme } from "./claudeProviderPresets";

export interface CodexProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  // Separate "get an API key" link for third-party providers
  apiKeyUrl?: string;
  auth: Record<string, any>; // Written to ~/.codex/auth.json
  config: string; // Written to ~/.codex/config.toml (TOML string)
  isOfficial?: boolean; // Official preset
  category?: ProviderCategory; // Category
  isCustomTemplate?: boolean; // Custom template
  // Candidate endpoints (for endpoint management and speed tests)
  endpointCandidates?: string[];
  // Visual theme
  theme?: PresetTheme;
  // Icon
  icon?: string; // Icon name
  iconColor?: string; // Icon color
}

/**
 * Build auth.json for a third-party provider
 */
export function generateThirdPartyAuth(apiKey: string): Record<string, any> {
  return {
    OPENAI_API_KEY: apiKey || "",
  };
}

/**
 * Build config.toml for a third-party provider
 */
export function generateThirdPartyConfig(
  providerName: string,
  baseUrl: string,
  modelName = "gpt-5.4",
): string {
  // Sanitize the provider name into a valid TOML key
  const cleanProviderName =
    providerName
      .toLowerCase()
      .replace(/[^a-z0-9_]/g, "_")
      .replace(/^_+|_+$/g, "") || "custom";

  return `model_provider = "${cleanProviderName}"
model = "${modelName}"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.${cleanProviderName}]
name = "${cleanProviderName}"
base_url = "${baseUrl}"
wire_api = "responses"
requires_openai_auth = true`;
}

export const codexProviderPresets: CodexProviderPreset[] = [
  {
    name: "OpenAI Official",
    websiteUrl: "https://chatgpt.com/codex",
    isOfficial: true,
    category: "official",
    auth: {},
    config: ``,
    theme: {
      icon: "codex",
      backgroundColor: "#1F2937", // gray-800
      textColor: "#FFFFFF",
    },
    icon: "openai",
    iconColor: "#00A67E",
  },
  {
    name: "Azure OpenAI",
    websiteUrl:
      "https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/codex",
    category: "third_party",
    isOfficial: true,
    auth: generateThirdPartyAuth(""),
    config: `model_provider = "azure"
model = "gpt-5.4"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.azure]
name = "Azure OpenAI"
base_url = "https://YOUR_RESOURCE_NAME.openai.azure.com/openai"
env_key = "OPENAI_API_KEY"
query_params = { "api-version" = "2025-04-01-preview" }
wire_api = "responses"
requires_openai_auth = true`,
    endpointCandidates: ["https://YOUR_RESOURCE_NAME.openai.azure.com/openai"],
    theme: {
      icon: "codex",
      backgroundColor: "#0078D4",
      textColor: "#FFFFFF",
    },
    icon: "azure",
    iconColor: "#0078D4",
  },
  {
    name: "AiHubMix",
    websiteUrl: "https://aihubmix.com",
    category: "aggregator",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "aihubmix",
      "https://aihubmix.com/v1",
      "gpt-5.4",
    ),
    endpointCandidates: [
      "https://aihubmix.com/v1",
      "https://api.aihubmix.com/v1",
    ],
  },
  {
    name: "DMXAPI",
    websiteUrl: "https://www.dmxapi.cn",
    category: "aggregator",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "dmxapi",
      "https://www.dmxapi.cn/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://www.dmxapi.cn/v1"],
  },
  {
    name: "PackyCode",
    websiteUrl: "https://www.packyapi.com",
    apiKeyUrl: "https://www.packyapi.com/register",
    category: "third_party",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "packycode",
      "https://www.packyapi.com/v1",
      "gpt-5.4",
    ),
    endpointCandidates: [
      "https://www.packyapi.com/v1",
      "https://api-slb.packyapi.com/v1",
    ],
    icon: "packycode",
  },
  {
    name: "Cubence",
    websiteUrl: "https://cubence.com",
    apiKeyUrl: "https://cubence.com/signup",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "cubence",
      "https://api.cubence.com/v1",
      "gpt-5.4",
    ),
    endpointCandidates: [
      "https://api.cubence.com/v1",
      "https://api-cf.cubence.com/v1",
      "https://api-dmit.cubence.com/v1",
      "https://api-bwg.cubence.com/v1",
    ],
    category: "third_party",
    icon: "cubence",
    iconColor: "#000000",
  },
  {
    name: "AIGoCode",
    websiteUrl: "https://aigocode.com",
    apiKeyUrl: "https://aigocode.com",
    category: "third_party",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "aigocode",
      "https://api.aigocode.com",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://api.aigocode.com"],
    icon: "aigocode",
    iconColor: "#5B7FFF",
  },
  {
    name: "RightCode",
    websiteUrl: "https://www.right.codes",
    apiKeyUrl: "https://www.right.codes/register",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "rightcode",
      "https://right.codes/codex/v1",
      "gpt-5.4",
    ),
    category: "third_party",
    icon: "rc",
    iconColor: "#E96B2C",
  },
  {
    name: "AICodeMirror",
    websiteUrl: "https://www.aicodemirror.com",
    apiKeyUrl: "https://www.aicodemirror.com/register",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "aicodemirror",
      "https://api.aicodemirror.com/api/codex/backend-api/codex",
      "gpt-5.4",
    ),
    endpointCandidates: [
      "https://api.aicodemirror.com/api/codex/backend-api/codex",
      "https://api.claudecode.net.cn/api/codex/backend-api/codex",
    ],
    icon: "aicodemirror",
    iconColor: "#000000",
  },
  {
    name: "AICoding",
    websiteUrl: "https://aicoding.sh",
    apiKeyUrl: "https://aicoding.sh",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "aicoding",
      "https://api.aicoding.sh",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://api.aicoding.sh"],
    icon: "aicoding",
    iconColor: "#000000",
  },
  {
    name: "CrazyRouter",
    websiteUrl: "https://www.crazyrouter.com",
    apiKeyUrl: "https://www.crazyrouter.com/register",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "crazyrouter",
      "https://crazyrouter.com/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://crazyrouter.com/v1"],
    icon: "crazyrouter",
    iconColor: "#000000",
  },
  {
    name: "SSSAiCode",
    websiteUrl: "https://www.sssaicode.com",
    apiKeyUrl: "https://www.sssaicode.com/register",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "sssaicode",
      "https://node-hk.sssaicode.com/api/v1",
      "gpt-5.4",
    ),
    endpointCandidates: [
      "https://node-hk.sssaicode.com/api/v1",
      "https://claude2.sssaicode.com/api/v1",
      "https://anti.sssaicode.com/api/v1",
    ],
    category: "third_party",
    icon: "sssaicode",
    iconColor: "#000000",
  },
  {
    name: "Compshare",
    nameKey: "providerForm.presets.ucloud",
    websiteUrl: "https://www.compshare.cn",
    apiKeyUrl: "https://www.compshare.cn/coding-plan",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "compshare",
      "https://api.modelverse.cn/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://api.modelverse.cn/v1"],
    category: "aggregator",
    icon: "ucloud",
    iconColor: "#000000",
  },
  {
    name: "Micu",
    websiteUrl: "https://www.openclaudecode.cn",
    apiKeyUrl: "https://www.openclaudecode.cn/register",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "micu",
      "https://www.openclaudecode.cn/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://www.openclaudecode.cn/v1"],
    category: "third_party",
    icon: "micu",
    iconColor: "#000000",
  },
  {
    name: "X-Code API",
    websiteUrl: "https://x-code.cc",
    apiKeyUrl: "https://x-code.cc",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "x-code",
      "https://x-code.cc/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://x-code.cc/v1"],
    category: "third_party",
    icon: "x-code",
    iconColor: "#000000",
  },
  {
    name: "CTok.ai",
    websiteUrl: "https://ctok.ai",
    apiKeyUrl: "https://ctok.ai",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "ctok",
      "https://api.ctok.ai/v1",
      "gpt-5.4",
    ),
    endpointCandidates: ["https://api.ctok.ai/v1"],
    category: "third_party",
    icon: "ctok",
    iconColor: "#000000",
  },
  {
    name: "OpenRouter",
    websiteUrl: "https://openrouter.ai",
    apiKeyUrl: "https://openrouter.ai/keys",
    auth: generateThirdPartyAuth(""),
    config: generateThirdPartyConfig(
      "openrouter",
      "https://openrouter.ai/api/v1",
      "gpt-5.4",
    ),
    category: "aggregator",
    icon: "openrouter",
    iconColor: "#6566F1",
  },
];
