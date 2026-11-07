import { useEffect, useState, useCallback } from "react";
import type { ProviderCategory } from "@/types";
import {
  getApiKeyFromConfig,
  setApiKeyInConfig,
  hasApiKeyField,
} from "@/utils/providerConfigUtils";

interface UseApiKeyStateProps {
  initialConfig?: string;
  onConfigChange: (config: string) => void;
  selectedPresetId: string | null;
  category?: ProviderCategory;
  appType?: string;
  apiKeyField?: string;
}

/**
 * Manages API key input state
 * Keeps the API key and the JSON config in sync
 */
export function useApiKeyState({
  initialConfig,
  onConfigChange,
  selectedPresetId,
  category,
  appType,
  apiKeyField,
}: UseApiKeyStateProps) {
  const [apiKey, setApiKey] = useState(() => {
    if (initialConfig) {
      return getApiKeyFromConfig(initialConfig, appType);
    }
    return "";
  });

  // When the config changes from outside (form.reset, reading the live config, etc.), sync it back into the API key state
  // - Only sync when the JSON parses, so the input does not flicker while the user is mid-edit
  useEffect(() => {
    if (!initialConfig) return;

    try {
      JSON.parse(initialConfig);
    } catch {
      return;
    }

    // Extract the API key from the config (empty string if missing)
    const extracted = getApiKeyFromConfig(initialConfig, appType);
    if (extracted !== apiKey) {
      setApiKey(extracted);
    }
  }, [initialConfig, appType, apiKey]);

  const handleApiKeyChange = useCallback(
    (key: string) => {
      setApiKey(key);

      const configString = setApiKeyInConfig(
        initialConfig || "{}",
        key.trim(),
        {
          // Only add missing fields in "create mode" for a "non-official category"
          // - Create mode: selectedPresetId !== null
          // - Non-official category: category !== undefined && category !== "official"
          // - Official category: do not create the field (the UI disables the input too)
          // - No category passed: do not create the field (avoids surprises)
          createIfMissing:
            selectedPresetId !== null &&
            category !== undefined &&
            category !== "official",
          appType,
          apiKeyField,
        },
      );

      onConfigChange(configString);
    },
    [
      initialConfig,
      selectedPresetId,
      category,
      appType,
      apiKeyField,
      onConfigChange,
    ],
  );

  const showApiKey = useCallback(
    (config: string, isEditMode: boolean) => {
      return (
        selectedPresetId !== null ||
        (isEditMode && hasApiKeyField(config, appType))
      );
    },
    [selectedPresetId, appType],
  );

  return {
    apiKey,
    setApiKey,
    handleApiKeyChange,
    showApiKey,
  };
}
