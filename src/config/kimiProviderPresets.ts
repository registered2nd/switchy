/**
 * Kimi Code 预设供应商配置模板
 *
 * Kimi 的供应商记录形如 `{ config, credentials }`：
 * - `config` 是完整的 `~/.kimi-code/config.toml` 文本
 * - `credentials` 是 `~/.kimi-code/credentials/kimi-code.json`（托管登录）或 null
 */
import { ProviderCategory } from "../types";
import type { PresetTheme } from "./claudeProviderPresets";

export type KimiProviderType = "openai" | "anthropic" | "kimi";

export interface KimiProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  // 第三方供应商可提供单独的获取 API Key 链接
  apiKeyUrl?: string;
  config: string; // 将写入 ~/.kimi-code/config.toml（TOML 字符串）
  credentials: Record<string, any> | null; // 将写入 credentials/kimi-code.json
  isOfficial?: boolean; // 标识是否为官方预设
  isPartner?: boolean; // 标识是否为商业合作伙伴
  partnerPromotionKey?: string; // 合作伙伴促销信息的 i18n key
  category?: ProviderCategory; // 分类
  isCustomTemplate?: boolean; // 标识是否为自定义模板
  // 请求地址候选列表（用于地址管理/测速）
  endpointCandidates?: string[];
  // 视觉主题配置
  theme?: PresetTheme;
  // 图标配置
  icon?: string;
  iconColor?: string;
}

const TOML_BARE_KEY = /^[A-Za-z0-9_-]+$/;

/** TOML 表头里的 key：裸键直接用，否则加双引号 */
export function quoteKimiTomlKey(key: string): string {
  return TOML_BARE_KEY.test(key) ? key : `"${key}"`;
}

/**
 * 生成第三方供应商的 config.toml
 *
 * `default_model` 使用 `<providerId>/<model>` 别名，别名表指回供应商。
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

/** 官方（托管登录）config.toml：kimi-code 0.26 写出的供应商与模型表 */
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
