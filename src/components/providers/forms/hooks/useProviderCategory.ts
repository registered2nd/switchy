import { useState, useEffect } from "react";
import type { ProviderCategory } from "@/types";
import type { AppId } from "@/lib/api";
import { providerPresets } from "@/config/claudeProviderPresets";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { geminiProviderPresets } from "@/config/geminiProviderPresets";
import { opencodeProviderPresets } from "@/config/opencodeProviderPresets";
import { kimiProviderPresets } from "@/config/kimiProviderPresets";

interface UseProviderCategoryProps {
  appId: AppId;
  selectedPresetId: string | null;
  isEditMode: boolean;
  initialCategory?: ProviderCategory;
}

/**
 * Manages provider category state
 * Updates the category from the selected preset
 */
export function useProviderCategory({
  appId,
  selectedPresetId,
  isEditMode,
  initialCategory,
}: UseProviderCategoryProps) {
  const [category, setCategory] = useState<ProviderCategory | undefined>(
    // Edit mode: use initialCategory
    isEditMode ? initialCategory : undefined,
  );

  useEffect(() => {
    // Edit mode: set only on init, never updated automatically afterwards
    if (isEditMode) {
      setCategory(initialCategory);
      return;
    }

    if (selectedPresetId === "custom") {
      setCategory("custom");
      return;
    }

    if (!selectedPresetId) return;

    // Extract the index from the preset ID
    const match = selectedPresetId.match(
      /^(claude|codex|gemini|kimi|opencode)-(\d+)$/,
    );
    if (!match) return;

    const [, type, indexStr] = match;
    const index = parseInt(indexStr, 10);

    if (type === "codex" && appId === "codex") {
      const preset = codexProviderPresets[index];
      if (preset) {
        setCategory(
          preset.category || (preset.isOfficial ? "official" : undefined),
        );
      }
    } else if (type === "claude" && appId === "claude") {
      const preset = providerPresets[index];
      if (preset) {
        setCategory(
          preset.category || (preset.isOfficial ? "official" : undefined),
        );
      }
    } else if (type === "gemini" && appId === "gemini") {
      const preset = geminiProviderPresets[index];
      if (preset) {
        setCategory(preset.category || undefined);
      }
    } else if (type === "opencode" && appId === "opencode") {
      const preset = opencodeProviderPresets[index];
      if (preset) {
        setCategory(preset.category || undefined);
      }
    } else if (type === "kimi" && appId === "kimi") {
      const preset = kimiProviderPresets[index];
      if (preset) {
        setCategory(
          preset.category || (preset.isOfficial ? "official" : undefined),
        );
      }
    }
  }, [appId, selectedPresetId, isEditMode, initialCategory]);

  return { category, setCategory };
}
