import { useMemo } from "react";
import type { AppId } from "@/lib/api";
import type { ProviderCategory } from "@/types";
import type { ProviderPreset } from "@/config/claudeProviderPresets";
import type { CodexProviderPreset } from "@/config/codexProviderPresets";
import type { GeminiProviderPreset } from "@/config/geminiProviderPresets";
import type { OpenCodeProviderPreset } from "@/config/opencodeProviderPresets";
import type { KimiProviderPreset } from "@/config/kimiProviderPresets";

type PresetEntry = {
  id: string;
  preset:
    | ProviderPreset
    | CodexProviderPreset
    | GeminiProviderPreset
    | OpenCodeProviderPreset
    | KimiProviderPreset;
};

interface UseApiKeyLinkProps {
  appId: AppId;
  category?: ProviderCategory;
  selectedPresetId: string | null;
  presetEntries: PresetEntry[];
  formWebsiteUrl: string;
}

/**
 * Controls whether the "get API key" link shows, and its URL
 */
export function useApiKeyLink({
  appId,
  category,
  selectedPresetId,
  presetEntries,
  formWebsiteUrl,
}: UseApiKeyLinkProps) {
  // Whether to show the "get API key" link
  const shouldShowApiKeyLink = useMemo(() => {
    return (
      category !== "official" &&
      (category === "cn_official" ||
        category === "aggregator" ||
        category === "third_party")
    );
  }, [category]);

  // Current preset entry
  const currentPresetEntry = useMemo(() => {
    if (selectedPresetId && selectedPresetId !== "custom") {
      return presetEntries.find((item) => item.id === selectedPresetId);
    }
    return undefined;
  }, [selectedPresetId, presetEntries]);

  // Current provider's website (for the API key link)
  const getWebsiteUrl = useMemo(() => {
    if (currentPresetEntry) {
      const preset = currentPresetEntry.preset;
      // For cn_official, aggregator and third_party, prefer apiKeyUrl (may carry referral parameters)
      if (
        preset.category === "cn_official" ||
        preset.category === "aggregator" ||
        preset.category === "third_party"
      ) {
        return preset.apiKeyUrl || preset.websiteUrl || "";
      }
      return preset.websiteUrl || "";
    }
    return formWebsiteUrl || "";
  }, [currentPresetEntry, formWebsiteUrl]);

  // Gemini presets name the auth they need (Google sign-in, PackyCode).
  const partnerPromotionKey = useMemo(() => {
    const preset = currentPresetEntry?.preset as
      | { partnerPromotionKey?: string }
      | undefined;
    return preset?.partnerPromotionKey;
  }, [currentPresetEntry]);

  return {
    shouldShowApiKeyLink:
      appId === "claude" ||
      appId === "codex" ||
      appId === "gemini" ||
      appId === "kimi" ||
      appId === "opencode"
        ? shouldShowApiKeyLink
        : false,
    websiteUrl: getWebsiteUrl,
    partnerPromotionKey,
  };
}
