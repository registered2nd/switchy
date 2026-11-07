/**
 * Universal provider presets
 *
 * A universal provider is one config shared across apps; changes sync to Claude, Codex and Gemini.
 * Meant for multi-protocol API gateways such as NewAPI.
 */

import type {
  UniversalProvider,
  UniversalProviderApps,
  UniversalProviderModels,
} from "@/types";

/**
 * Universal provider preset
 */
export interface UniversalProviderPreset {
  /** Preset name */
  name: string;
  /** Provider type */
  providerType: string;
  /** Apps enabled by default */
  defaultApps: UniversalProviderApps;
  /** Default models */
  defaultModels: UniversalProviderModels;
  /** Website URL */
  websiteUrl?: string;
  /** Icon name */
  icon?: string;
  /** Icon color */
  iconColor?: string;
  /** Description */
  description?: string;
  /** Custom template (fully user-defined) */
  isCustomTemplate?: boolean;
}

/**
 * NewAPI default models
 */
const NEWAPI_DEFAULT_MODELS: UniversalProviderModels = {
  claude: {
    model: "claude-sonnet-4-20250514",
    haikuModel: "claude-haiku-4-20250514",
    sonnetModel: "claude-sonnet-4-20250514",
    opusModel: "claude-sonnet-4-20250514",
  },
  codex: {
    model: "gpt-5.4",
    reasoningEffort: "high",
  },
  gemini: {
    model: "gemini-2.5-pro",
  },
};

/**
 * Universal provider presets
 */
export const universalProviderPresets: UniversalProviderPreset[] = [
  {
    name: "NewAPI",
    providerType: "newapi",
    defaultApps: {
      claude: true,
      codex: true,
      gemini: true,
    },
    defaultModels: NEWAPI_DEFAULT_MODELS,
    websiteUrl: "https://www.newapi.pro",
    icon: "newapi",
    iconColor: "#00A67E",
    description:
      "Self-hosted API gateway for Anthropic, OpenAI, Gemini and other protocols",
  },
  {
    name: "Custom Gateway",
    providerType: "custom_gateway",
    defaultApps: {
      claude: true,
      codex: true,
      gemini: true,
    },
    defaultModels: NEWAPI_DEFAULT_MODELS,
    icon: "openai",
    iconColor: "#6366F1",
    description: "An API gateway you configure yourself",
    isCustomTemplate: true,
  },
];

/**
 * Create a universal provider from a preset
 */
export function createUniversalProviderFromPreset(
  preset: UniversalProviderPreset,
  id: string,
  baseUrl: string,
  apiKey: string,
  customName?: string,
): UniversalProvider {
  return {
    id,
    name: customName || preset.name,
    providerType: preset.providerType,
    apps: { ...preset.defaultApps },
    baseUrl,
    apiKey,
    models: JSON.parse(JSON.stringify(preset.defaultModels)), // Deep copy
    websiteUrl: preset.websiteUrl,
    icon: preset.icon,
    iconColor: preset.iconColor,
    createdAt: Date.now(),
  };
}

/**
 * Display name of a preset (for the UI)
 */
export function getPresetDisplayName(preset: UniversalProviderPreset): string {
  return preset.name;
}

/**
 * Find a preset by type
 */
export function findPresetByType(
  providerType: string,
): UniversalProviderPreset | undefined {
  return universalProviderPresets.find((p) => p.providerType === providerType);
}
