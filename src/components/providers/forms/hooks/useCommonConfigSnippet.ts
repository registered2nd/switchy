import { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  updateCommonConfigSnippet,
  hasCommonConfigSnippet,
  validateJsonConfig,
} from "@/utils/providerConfigUtils";
import { configApi } from "@/lib/api";

const LEGACY_STORAGE_KEY = "switchy:common-config-snippet";
const DEFAULT_COMMON_CONFIG_SNIPPET = `{
  "includeCoAuthoredBy": false
}`;

interface UseCommonConfigSnippetProps {
  settingsConfig: string;
  onConfigChange: (config: string) => void;
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
  initialEnabled?: boolean;
  selectedPresetId?: string;
  /** When false, the hook skips all logic and returns disabled state. Default: true */
  enabled?: boolean;
}

/**
 * Manages the Claude common config snippet
 * Reads and saves it in config.json, migrating from localStorage when needed
 */
export function useCommonConfigSnippet({
  settingsConfig,
  onConfigChange,
  initialData,
  initialEnabled,
  selectedPresetId,
  enabled = true,
}: UseCommonConfigSnippetProps) {
  const { t } = useTranslation();
  const [useCommonConfig, setUseCommonConfig] = useState(false);
  const [commonConfigSnippet, setCommonConfigSnippetState] = useState<string>(
    DEFAULT_COMMON_CONFIG_SNIPPET,
  );
  const [commonConfigError, setCommonConfigError] = useState("");
  const [isLoading, setIsLoading] = useState(true);
  const [isExtracting, setIsExtracting] = useState(false);

  // Tracks whether an update is coming from the common config
  const isUpdatingFromCommonConfig = useRef(false);
  // Tracks whether create mode has set the default checkbox state
  const hasInitializedNewMode = useRef(false);
  // Tracks whether edit mode has initialized the explicit toggle/preview
  const hasInitializedEditMode = useRef(false);

  // Reset the init flags when the preset changes so the new preset runs initialization again
  useEffect(() => {
    if (!enabled) return;
    hasInitializedNewMode.current = false;
    hasInitializedEditMode.current = false;
  }, [selectedPresetId, enabled, initialEnabled]);

  // Init: load from config.json, migrating from localStorage if needed
  useEffect(() => {
    if (!enabled) {
      setIsLoading(false);
      return;
    }
    let mounted = true;

    const loadSnippet = async () => {
      try {
        // Load through the shared API
        const snippet = await configApi.getCommonConfigSnippet("claude");

        if (snippet && snippet.trim()) {
          if (mounted) {
            setCommonConfigSnippetState(snippet);
          }
        } else {
          // If config.json has none, try migrating from localStorage
          if (typeof window !== "undefined") {
            try {
              const legacySnippet =
                window.localStorage.getItem(LEGACY_STORAGE_KEY);
              if (legacySnippet && legacySnippet.trim()) {
                // Migrate to config.json
                await configApi.setCommonConfigSnippet("claude", legacySnippet);
                if (mounted) {
                  setCommonConfigSnippetState(legacySnippet);
                }
                // Clear localStorage
                window.localStorage.removeItem(LEGACY_STORAGE_KEY);
                console.log(
                  "[migration] Claude common config moved from localStorage to config.json",
                );
              }
            } catch (e) {
              console.warn(
                "[migration] Migration from localStorage failed:",
                e,
              );
            }
          }
        }
      } catch (error) {
        console.error("Failed to load common config:", error);
      } finally {
        if (mounted) {
          setIsLoading(false);
        }
      }
    };

    loadSnippet();

    return () => {
      mounted = false;
    };
  }, [enabled]);

  // On init, check for the common config snippet (edit mode)
  useEffect(() => {
    if (!enabled) return;
    if (initialData && !isLoading) {
      const configString = JSON.stringify(initialData.settingsConfig, null, 2);
      const inferredHasCommon = hasCommonConfigSnippet(
        configString,
        commonConfigSnippet,
      );
      const hasCommon = initialEnabled ?? inferredHasCommon;
      setUseCommonConfig(hasCommon);

      if (hasCommon && !inferredHasCommon && !hasInitializedEditMode.current) {
        hasInitializedEditMode.current = true;
        const { updatedConfig, error } = updateCommonConfigSnippet(
          settingsConfig,
          commonConfigSnippet,
          true,
        );
        if (!error) {
          isUpdatingFromCommonConfig.current = true;
          onConfigChange(updatedConfig);
          setTimeout(() => {
            isUpdatingFromCommonConfig.current = false;
          }, 0);
        }
      } else {
        hasInitializedEditMode.current = true;
      }
    }
  }, [
    enabled,
    initialData,
    initialEnabled,
    commonConfigSnippet,
    isLoading,
    onConfigChange,
    settingsConfig,
  ]);

  // Create mode: enable by default if the common config snippet exists and is valid
  useEffect(() => {
    if (!enabled) return;
    // Only in create mode, after loading, and not yet initialized
    if (!initialData && !isLoading && !hasInitializedNewMode.current) {
      hasInitializedNewMode.current = true;

      // Check that the snippet has real content
      try {
        const snippetObj = JSON.parse(commonConfigSnippet);
        const hasContent = Object.keys(snippetObj).length > 0;
        if (hasContent) {
          setUseCommonConfig(true);
          // Merge the common config into the current config
          const { updatedConfig, error } = updateCommonConfigSnippet(
            settingsConfig,
            commonConfigSnippet,
            true,
          );
          if (!error) {
            isUpdatingFromCommonConfig.current = true;
            onConfigChange(updatedConfig);
            setTimeout(() => {
              isUpdatingFromCommonConfig.current = false;
            }, 0);
          }
        }
      } catch {
        // ignore parse error
      }
    }
  }, [
    enabled,
    initialData,
    commonConfigSnippet,
    isLoading,
    settingsConfig,
    onConfigChange,
  ]);

  // Handle the common config toggle
  const handleCommonConfigToggle = useCallback(
    (checked: boolean) => {
      const { updatedConfig, error: snippetError } = updateCommonConfigSnippet(
        settingsConfig,
        commonConfigSnippet,
        checked,
      );

      if (snippetError) {
        setCommonConfigError(snippetError);
        setUseCommonConfig(false);
        return;
      }

      setCommonConfigError("");
      setUseCommonConfig(checked);
      // Mark that the update comes from the common config
      isUpdatingFromCommonConfig.current = true;
      onConfigChange(updatedConfig);
      // Reset the flag on the next tick
      setTimeout(() => {
        isUpdatingFromCommonConfig.current = false;
      }, 0);
    },
    [settingsConfig, commonConfigSnippet, onConfigChange],
  );

  // Handle common config snippet changes
  const handleCommonConfigSnippetChange = useCallback(
    (value: string) => {
      const previousSnippet = commonConfigSnippet;
      setCommonConfigSnippetState(value);

      if (!value.trim()) {
        setCommonConfigError("");
        // Save to config.json (cleared)
        configApi
          .setCommonConfigSnippet("claude", "")
          .catch((error: unknown) => {
            console.error("Failed to save common config:", error);
            setCommonConfigError(
              t("claudeConfig.saveFailed", { error: String(error) }),
            );
          });

        if (useCommonConfig) {
          const { updatedConfig } = updateCommonConfigSnippet(
            settingsConfig,
            previousSnippet,
            false,
          );
          onConfigChange(updatedConfig);
          setUseCommonConfig(false);
        }
        return;
      }

      // Validate JSON
      const validationError = validateJsonConfig(
        value,
        t("providerForm.fieldCommonSnippet"),
      );
      if (validationError) {
        setCommonConfigError(validationError);
      } else {
        setCommonConfigError("");
        // Save to config.json
        configApi
          .setCommonConfigSnippet("claude", value)
          .catch((error: unknown) => {
            console.error("Failed to save common config:", error);
            setCommonConfigError(
              t("claudeConfig.saveFailed", { error: String(error) }),
            );
          });
      }

      // If the common config is enabled and valid, swap in the latest snippet
      if (useCommonConfig && !validationError) {
        const removeResult = updateCommonConfigSnippet(
          settingsConfig,
          previousSnippet,
          false,
        );
        if (removeResult.error) {
          setCommonConfigError(removeResult.error);
          return;
        }
        const addResult = updateCommonConfigSnippet(
          removeResult.updatedConfig,
          value,
          true,
        );

        if (addResult.error) {
          setCommonConfigError(addResult.error);
          return;
        }

        // Mark that the update comes from the common config so the state check does not fire
        isUpdatingFromCommonConfig.current = true;
        onConfigChange(addResult.updatedConfig);
        // Reset the flag on the next tick
        setTimeout(() => {
          isUpdatingFromCommonConfig.current = false;
        }, 0);
      }
    },
    [commonConfigSnippet, settingsConfig, useCommonConfig, onConfigChange],
  );

  // When the config changes, check whether it contains the common config (skipped while the common config itself is updating)
  useEffect(() => {
    if (!enabled) return;
    if (isUpdatingFromCommonConfig.current || isLoading) {
      return;
    }
    const hasCommon = hasCommonConfigSnippet(
      settingsConfig,
      commonConfigSnippet,
    );
    setUseCommonConfig(hasCommon);
  }, [enabled, settingsConfig, commonConfigSnippet, isLoading]);

  // Extract the common config snippet from the editor's current content
  const handleExtract = useCallback(async () => {
    setIsExtracting(true);
    setCommonConfigError("");

    try {
      const extracted = await configApi.extractCommonConfigSnippet("claude", {
        settingsConfig,
      });

      if (!extracted || extracted === "{}") {
        setCommonConfigError(t("claudeConfig.extractNoCommonConfig"));
        return;
      }

      // Validate JSON
      const validationError = validateJsonConfig(
        extracted,
        t("providerForm.fieldExtractedConfig"),
      );
      if (validationError) {
        setCommonConfigError(validationError);
        return;
      }

      // Update snippet state
      setCommonConfigSnippetState(extracted);

      // Save to the backend
      await configApi.setCommonConfigSnippet("claude", extracted);
    } catch (error) {
      console.error("Failed to extract common config:", error);
      setCommonConfigError(
        t("claudeConfig.extractFailed", { error: String(error) }),
      );
    } finally {
      setIsExtracting(false);
    }
  }, [settingsConfig, t]);

  return {
    useCommonConfig,
    commonConfigSnippet,
    commonConfigError,
    isLoading,
    isExtracting,
    handleCommonConfigToggle,
    handleCommonConfigSnippetChange,
    handleExtract,
  };
}
