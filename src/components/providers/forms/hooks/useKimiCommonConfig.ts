import { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { parse as parseToml } from "smol-toml";
import {
  updateTomlCommonConfigSnippet,
  hasTomlCommonConfigSnippet,
} from "@/utils/providerConfigUtils";
import { configApi } from "@/lib/api";
import { normalizeTomlText } from "@/utils/textNormalization";

const LEGACY_STORAGE_KEY = "switchy:kimi-common-config-snippet";
const DEFAULT_KIMI_COMMON_CONFIG_SNIPPET = `# Common Kimi config
# Add your common TOML configuration here`;

interface UseKimiCommonConfigProps {
  kimiConfig: string;
  onConfigChange: (config: string) => void;
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
  initialEnabled?: boolean;
  selectedPresetId?: string;
  /** Only enabled for the Kimi form; other apps do not read or write the common config when mounted */
  enabled?: boolean;
}

/**
 * Manages the Kimi common config snippet (TOML)
 * Reads and saves it in config.json, migrating from localStorage when needed
 */
export function useKimiCommonConfig({
  kimiConfig,
  onConfigChange,
  initialData,
  initialEnabled,
  selectedPresetId,
  enabled = true,
}: UseKimiCommonConfigProps) {
  const { t } = useTranslation();
  const [useCommonConfig, setUseCommonConfig] = useState(false);
  const [commonConfigSnippet, setCommonConfigSnippetState] = useState<string>(
    DEFAULT_KIMI_COMMON_CONFIG_SNIPPET,
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
    hasInitializedNewMode.current = false;
    hasInitializedEditMode.current = false;
  }, [selectedPresetId, initialEnabled]);

  const parseCommonConfigSnippet = useCallback((snippetString: string) => {
    const trimmed = snippetString.trim();
    if (!trimmed) {
      return {
        hasContent: false,
      };
    }

    try {
      const parsed = parseToml(normalizeTomlText(snippetString)) as Record<
        string,
        unknown
      >;
      return {
        hasContent: Object.keys(parsed).length > 0,
      };
    } catch (error) {
      return {
        hasContent: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }, []);

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
        const snippet = await configApi.getCommonConfigSnippet("kimi");

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
                await configApi.setCommonConfigSnippet("kimi", legacySnippet);
                if (mounted) {
                  setCommonConfigSnippetState(legacySnippet);
                }
                // Clear localStorage
                window.localStorage.removeItem(LEGACY_STORAGE_KEY);
                console.log(
                  "[migration] Kimi common config moved from localStorage to config.json",
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
        console.error("Failed to load Kimi common config:", error);
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
    if (
      !enabled ||
      !initialData?.settingsConfig ||
      isLoading ||
      hasInitializedEditMode.current
    ) {
      return;
    }

    hasInitializedEditMode.current = true;

    const parsedSnippet = parseCommonConfigSnippet(commonConfigSnippet);
    if (parsedSnippet.error) {
      if (commonConfigSnippet.trim()) {
        setCommonConfigError(parsedSnippet.error);
      }
      setUseCommonConfig(false);
      return;
    }

    const config =
      typeof initialData.settingsConfig.config === "string"
        ? initialData.settingsConfig.config
        : "";
    const inferredHasCommon = hasTomlCommonConfigSnippet(
      config,
      commonConfigSnippet,
    );
    const hasCommon = initialEnabled ?? inferredHasCommon;

    if (hasCommon && !inferredHasCommon) {
      const { updatedConfig, error } = updateTomlCommonConfigSnippet(
        kimiConfig,
        commonConfigSnippet,
        true,
      );
      if (error) {
        setCommonConfigError(error);
        setUseCommonConfig(false);
        return;
      }

      setCommonConfigError("");
      setUseCommonConfig(true);
      isUpdatingFromCommonConfig.current = true;
      onConfigChange(updatedConfig);
      setTimeout(() => {
        isUpdatingFromCommonConfig.current = false;
      }, 0);
      return;
    }

    setCommonConfigError("");
    setUseCommonConfig(hasCommon);
  }, [
    kimiConfig,
    commonConfigSnippet,
    initialData,
    initialEnabled,
    isLoading,
    onConfigChange,
    parseCommonConfigSnippet,
  ]);

  // Create mode: enable by default if the common config snippet exists and is valid
  useEffect(() => {
    if (!enabled || initialData || isLoading || hasInitializedNewMode.current) {
      return;
    }

    hasInitializedNewMode.current = true;

    const parsedSnippet = parseCommonConfigSnippet(commonConfigSnippet);
    if (parsedSnippet.error) {
      if (commonConfigSnippet.trim()) {
        setCommonConfigError(parsedSnippet.error);
      }
      setUseCommonConfig(false);
      return;
    }
    if (!parsedSnippet.hasContent) {
      return;
    }

    const { updatedConfig, error } = updateTomlCommonConfigSnippet(
      kimiConfig,
      commonConfigSnippet,
      true,
    );
    if (error) {
      setCommonConfigError(error);
      setUseCommonConfig(false);
      return;
    }

    setCommonConfigError("");
    setUseCommonConfig(true);
    isUpdatingFromCommonConfig.current = true;
    onConfigChange(updatedConfig);
    setTimeout(() => {
      isUpdatingFromCommonConfig.current = false;
    }, 0);
  }, [
    initialData,
    commonConfigSnippet,
    isLoading,
    kimiConfig,
    onConfigChange,
    parseCommonConfigSnippet,
  ]);

  // Handle the common config toggle
  const handleCommonConfigToggle = useCallback(
    (checked: boolean) => {
      const parsedSnippet = parseCommonConfigSnippet(commonConfigSnippet);
      if (parsedSnippet.error) {
        setCommonConfigError(parsedSnippet.error);
        setUseCommonConfig(false);
        return;
      }
      if (!parsedSnippet.hasContent) {
        setCommonConfigError(
          t("kimiConfig.noCommonConfigToApply", {
            defaultValue: "Common config snippet is empty; nothing to apply.",
          }),
        );
        setUseCommonConfig(false);
        return;
      }

      const { updatedConfig, error: snippetError } =
        updateTomlCommonConfigSnippet(kimiConfig, commonConfigSnippet, checked);

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
    [
      kimiConfig,
      commonConfigSnippet,
      onConfigChange,
      parseCommonConfigSnippet,
      t,
    ],
  );

  // Handle common config snippet changes
  const handleCommonConfigSnippetChange = useCallback(
    (value: string): boolean => {
      const previousSnippet = commonConfigSnippet;

      if (!value.trim()) {
        setCommonConfigError("");

        if (useCommonConfig) {
          const previousParsed = parseCommonConfigSnippet(previousSnippet);
          let updatedConfig = kimiConfig;

          if (!previousParsed.error && previousParsed.hasContent) {
            const removeResult = updateTomlCommonConfigSnippet(
              kimiConfig,
              previousSnippet,
              false,
            );
            if (removeResult.error) {
              setCommonConfigError(removeResult.error);
              return false;
            }
            updatedConfig = removeResult.updatedConfig;
          }

          onConfigChange(updatedConfig);
          setUseCommonConfig(false);
        }

        setCommonConfigSnippetState("");
        configApi.setCommonConfigSnippet("kimi", "").catch((error: unknown) => {
          console.error("Failed to save Kimi common config:", error);
          setCommonConfigError(
            t("kimiConfig.saveFailed", { error: String(error) }),
          );
        });
        return true;
      }

      const parsedNextSnippet = parseCommonConfigSnippet(value);
      if (parsedNextSnippet.error) {
        setCommonConfigError(parsedNextSnippet.error);
        return false;
      }

      // If the common config is enabled, swap in the latest snippet
      if (useCommonConfig) {
        let nextConfig = kimiConfig;
        const previousParsed = parseCommonConfigSnippet(previousSnippet);

        if (!previousParsed.error && previousParsed.hasContent) {
          const removeResult = updateTomlCommonConfigSnippet(
            kimiConfig,
            previousSnippet,
            false,
          );
          if (removeResult.error) {
            setCommonConfigError(removeResult.error);
            return false;
          }
          nextConfig = removeResult.updatedConfig;
        }

        const addResult = updateTomlCommonConfigSnippet(
          nextConfig,
          value,
          true,
        );

        if (addResult.error) {
          setCommonConfigError(addResult.error);
          return false;
        }

        // Mark that the update comes from the common config so the state check does not fire
        isUpdatingFromCommonConfig.current = true;
        onConfigChange(addResult.updatedConfig);
        // Reset the flag on the next tick
        setTimeout(() => {
          isUpdatingFromCommonConfig.current = false;
        }, 0);
      }

      setCommonConfigError("");
      setCommonConfigSnippetState(value);
      configApi
        .setCommonConfigSnippet("kimi", value)
        .catch((error: unknown) => {
          console.error("Failed to save Kimi common config:", error);
          setCommonConfigError(
            t("kimiConfig.saveFailed", { error: String(error) }),
          );
        });

      return true;
    },
    [
      commonConfigSnippet,
      kimiConfig,
      onConfigChange,
      parseCommonConfigSnippet,
      t,
      useCommonConfig,
    ],
  );

  // When the config changes, check whether it contains the common config (skipped while the common config itself is updating)
  useEffect(() => {
    if (!enabled || isUpdatingFromCommonConfig.current || isLoading) {
      return;
    }
    const parsedSnippet = parseCommonConfigSnippet(commonConfigSnippet);
    if (parsedSnippet.error) {
      setUseCommonConfig(false);
      return;
    }
    const hasCommon = hasTomlCommonConfigSnippet(
      kimiConfig,
      commonConfigSnippet,
    );
    setUseCommonConfig(hasCommon);
  }, [kimiConfig, commonConfigSnippet, isLoading, parseCommonConfigSnippet]);

  // Extract the common config snippet from the editor's current content
  const handleExtract = useCallback(async () => {
    setIsExtracting(true);
    setCommonConfigError("");

    try {
      const extracted = await configApi.extractCommonConfigSnippet("kimi", {
        settingsConfig: JSON.stringify({
          config: kimiConfig ?? "",
        }),
      });

      if (!extracted || !extracted.trim()) {
        setCommonConfigError(t("kimiConfig.extractNoCommonConfig"));
        return;
      }

      // Update snippet state
      setCommonConfigSnippetState(extracted);

      // Save to the backend
      await configApi.setCommonConfigSnippet("kimi", extracted);
    } catch (error) {
      console.error("Failed to extract Kimi common config:", error);
      setCommonConfigError(
        t("kimiConfig.extractFailed", { error: String(error) }),
      );
    } finally {
      setIsExtracting(false);
    }
  }, [kimiConfig, t]);

  const clearCommonConfigError = useCallback(() => {
    setCommonConfigError("");
  }, []);

  return {
    useCommonConfig,
    commonConfigSnippet,
    commonConfigError,
    isLoading,
    isExtracting,
    handleCommonConfigToggle,
    handleCommonConfigSnippetChange,
    handleExtract,
    clearCommonConfigError,
  };
}
