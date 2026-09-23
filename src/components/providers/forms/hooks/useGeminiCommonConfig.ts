import { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { configApi } from "@/lib/api";

const LEGACY_STORAGE_KEY = "switchy:gemini-common-config-snippet";
const DEFAULT_GEMINI_COMMON_CONFIG_SNIPPET = "{}";

const GEMINI_COMMON_ENV_FORBIDDEN_KEYS = [
  "GOOGLE_GEMINI_BASE_URL",
  "GEMINI_API_KEY",
] as const;
type GeminiForbiddenEnvKey = (typeof GEMINI_COMMON_ENV_FORBIDDEN_KEYS)[number];

interface UseGeminiCommonConfigProps {
  envValue: string;
  onEnvChange: (env: string) => void;
  envStringToObj: (envString: string) => Record<string, string>;
  envObjToString: (envObj: Record<string, unknown>) => string;
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
  initialEnabled?: boolean;
  selectedPresetId?: string;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.prototype.toString.call(value) === "[object Object]"
  );
}

/**
 * Manages the Gemini common config snippet (JSON)
 * Written to Gemini's .env, excluding these sensitive fields:
 * - GOOGLE_GEMINI_BASE_URL
 * - GEMINI_API_KEY
 */
export function useGeminiCommonConfig({
  envValue,
  onEnvChange,
  envStringToObj,
  envObjToString,
  initialData,
  initialEnabled,
  selectedPresetId,
}: UseGeminiCommonConfigProps) {
  const { t } = useTranslation();
  const [useCommonConfig, setUseCommonConfig] = useState(false);
  const [commonConfigSnippet, setCommonConfigSnippetState] = useState<string>(
    DEFAULT_GEMINI_COMMON_CONFIG_SNIPPET,
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

  const parseSnippetEnv = useCallback(
    (
      snippetString: string,
    ): { env: Record<string, string>; error?: string } => {
      const trimmed = snippetString.trim();
      if (!trimmed) {
        return { env: {} };
      }

      let parsed: unknown;
      try {
        parsed = JSON.parse(trimmed);
      } catch {
        return { env: {}, error: t("geminiConfig.invalidJsonFormat") };
      }

      if (!isPlainObject(parsed)) {
        return { env: {}, error: t("geminiConfig.invalidJsonFormat") };
      }

      const keys = Object.keys(parsed);
      const forbiddenKeys = keys.filter((key) =>
        GEMINI_COMMON_ENV_FORBIDDEN_KEYS.includes(key as GeminiForbiddenEnvKey),
      );
      if (forbiddenKeys.length > 0) {
        return {
          env: {},
          error: t("geminiConfig.commonConfigInvalidKeys", {
            keys: forbiddenKeys.join(", "),
          }),
        };
      }

      const env: Record<string, string> = {};
      for (const [key, value] of Object.entries(parsed)) {
        if (typeof value !== "string") {
          return {
            env: {},
            error: t("geminiConfig.commonConfigInvalidValues"),
          };
        }
        const normalized = value.trim();
        if (!normalized) continue;
        env[key] = normalized;
      }

      return { env };
    },
    [t],
  );

  const hasEnvCommonConfigSnippet = useCallback(
    (envObj: Record<string, string>, snippetEnv: Record<string, string>) => {
      const entries = Object.entries(snippetEnv);
      if (entries.length === 0) return false;
      return entries.every(([key, value]) => envObj[key] === value);
    },
    [],
  );

  const applySnippetToEnv = useCallback(
    (envObj: Record<string, string>, snippetEnv: Record<string, string>) => {
      const updated = { ...envObj };
      for (const [key, value] of Object.entries(snippetEnv)) {
        if (typeof value === "string") {
          updated[key] = value;
        }
      }
      return updated;
    },
    [],
  );

  const removeSnippetFromEnv = useCallback(
    (envObj: Record<string, string>, snippetEnv: Record<string, string>) => {
      const updated = { ...envObj };
      for (const [key, value] of Object.entries(snippetEnv)) {
        if (typeof value === "string" && updated[key] === value) {
          delete updated[key];
        }
      }
      return updated;
    },
    [],
  );

  // Init: load from config.json, migrating from localStorage if needed
  useEffect(() => {
    let mounted = true;

    const loadSnippet = async () => {
      try {
        // Load through the shared API
        const snippet = await configApi.getCommonConfigSnippet("gemini");

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
                const parsed = parseSnippetEnv(legacySnippet);
                if (parsed.error) {
                  console.warn(
                    "[migration] Legacy Gemini common config snippet does not match the current rules; skipping migration",
                  );
                  return;
                }
                // Migrate to config.json
                await configApi.setCommonConfigSnippet("gemini", legacySnippet);
                if (mounted) {
                  setCommonConfigSnippetState(legacySnippet);
                }
                // Clear localStorage
                window.localStorage.removeItem(LEGACY_STORAGE_KEY);
                console.log(
                  "[migration] Gemini common config moved from localStorage to config.json",
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
        console.error("Failed to load Gemini common config:", error);
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
  }, [parseSnippetEnv]);

  // On init, check for the common config snippet (edit mode)
  useEffect(() => {
    if (
      !initialData?.settingsConfig ||
      isLoading ||
      hasInitializedEditMode.current
    ) {
      return;
    }

    hasInitializedEditMode.current = true;

    try {
      const env =
        isPlainObject(initialData.settingsConfig.env) &&
        Object.keys(initialData.settingsConfig.env).length > 0
          ? (initialData.settingsConfig.env as Record<string, string>)
          : {};
      const parsed = parseSnippetEnv(commonConfigSnippet);
      if (parsed.error) {
        if (commonConfigSnippet.trim()) {
          setCommonConfigError(parsed.error);
        }
        setUseCommonConfig(false);
        return;
      }
      const inferredHasCommon = hasEnvCommonConfigSnippet(
        env,
        parsed.env as Record<string, string>,
      );
      const hasCommon = initialEnabled ?? inferredHasCommon;

      if (hasCommon && !inferredHasCommon) {
        const currentEnv = envStringToObj(envValue);
        const merged = applySnippetToEnv(currentEnv, parsed.env);
        const nextEnvString = envObjToString(merged);

        setCommonConfigError("");
        setUseCommonConfig(true);
        isUpdatingFromCommonConfig.current = true;
        onEnvChange(nextEnvString);
        setTimeout(() => {
          isUpdatingFromCommonConfig.current = false;
        }, 0);
        return;
      }

      setCommonConfigError("");
      setUseCommonConfig(hasCommon);
    } catch {
      // ignore parse error
    }
  }, [
    applySnippetToEnv,
    commonConfigSnippet,
    envObjToString,
    envStringToObj,
    envValue,
    hasEnvCommonConfigSnippet,
    initialData,
    initialEnabled,
    isLoading,
    onEnvChange,
    parseSnippetEnv,
  ]);

  // Create mode: enable by default if the common config snippet exists and is valid
  useEffect(() => {
    if (initialData || isLoading || hasInitializedNewMode.current) {
      return;
    }

    hasInitializedNewMode.current = true;

    const parsed = parseSnippetEnv(commonConfigSnippet);
    if (parsed.error) {
      if (commonConfigSnippet.trim()) {
        setCommonConfigError(parsed.error);
      }
      setUseCommonConfig(false);
      return;
    }
    const hasContent = Object.keys(parsed.env).length > 0;
    if (!hasContent) return;

    setCommonConfigError("");
    setUseCommonConfig(true);
    const currentEnv = envStringToObj(envValue);
    const merged = applySnippetToEnv(currentEnv, parsed.env);
    const nextEnvString = envObjToString(merged);

    isUpdatingFromCommonConfig.current = true;
    onEnvChange(nextEnvString);
    setTimeout(() => {
      isUpdatingFromCommonConfig.current = false;
    }, 0);
  }, [
    initialData,
    isLoading,
    commonConfigSnippet,
    envValue,
    envStringToObj,
    envObjToString,
    applySnippetToEnv,
    onEnvChange,
    parseSnippetEnv,
  ]);

  // Handle the common config toggle
  const handleCommonConfigToggle = useCallback(
    (checked: boolean) => {
      const parsed = parseSnippetEnv(commonConfigSnippet);
      if (parsed.error) {
        setCommonConfigError(parsed.error);
        setUseCommonConfig(false);
        return;
      }
      if (Object.keys(parsed.env).length === 0) {
        setCommonConfigError(t("geminiConfig.noCommonConfigToApply"));
        setUseCommonConfig(false);
        return;
      }

      const currentEnv = envStringToObj(envValue);
      const updatedEnvObj = checked
        ? applySnippetToEnv(currentEnv, parsed.env)
        : removeSnippetFromEnv(currentEnv, parsed.env);

      setCommonConfigError("");
      setUseCommonConfig(checked);

      isUpdatingFromCommonConfig.current = true;
      onEnvChange(envObjToString(updatedEnvObj));
      setTimeout(() => {
        isUpdatingFromCommonConfig.current = false;
      }, 0);
    },
    [
      applySnippetToEnv,
      commonConfigSnippet,
      envObjToString,
      envStringToObj,
      envValue,
      onEnvChange,
      parseSnippetEnv,
      removeSnippetFromEnv,
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
          const parsedPrevious = parseSnippetEnv(previousSnippet);
          if (
            !parsedPrevious.error &&
            Object.keys(parsedPrevious.env).length > 0
          ) {
            const currentEnv = envStringToObj(envValue);
            const updatedEnv = removeSnippetFromEnv(
              currentEnv,
              parsedPrevious.env,
            );
            onEnvChange(envObjToString(updatedEnv));
          }
          setUseCommonConfig(false);
        }

        setCommonConfigSnippetState("");
        configApi
          .setCommonConfigSnippet("gemini", "")
          .catch((error: unknown) => {
            console.error("Failed to save Gemini common config:", error);
            setCommonConfigError(
              t("geminiConfig.saveFailed", { error: String(error) }),
            );
          });
        return true;
      }

      // Validate JSON
      const parsed = parseSnippetEnv(value);
      if (parsed.error) {
        setCommonConfigError(parsed.error);
        return false;
      }

      // If the common config is enabled, swap in the latest snippet
      if (useCommonConfig) {
        const prevParsed = parseSnippetEnv(previousSnippet);
        const prevEnv = prevParsed.error ? {} : prevParsed.env;
        const nextEnv = parsed.env;
        const currentEnv = envStringToObj(envValue);

        const withoutOld =
          Object.keys(prevEnv).length > 0
            ? removeSnippetFromEnv(currentEnv, prevEnv)
            : currentEnv;
        const withNew =
          Object.keys(nextEnv).length > 0
            ? applySnippetToEnv(withoutOld, nextEnv)
            : withoutOld;

        isUpdatingFromCommonConfig.current = true;
        onEnvChange(envObjToString(withNew));
        setTimeout(() => {
          isUpdatingFromCommonConfig.current = false;
        }, 0);
      }

      setCommonConfigError("");
      setCommonConfigSnippetState(value);
      configApi
        .setCommonConfigSnippet("gemini", value)
        .catch((error: unknown) => {
          console.error("Failed to save Gemini common config:", error);
          setCommonConfigError(
            t("geminiConfig.saveFailed", { error: String(error) }),
          );
        });

      return true;
    },
    [
      applySnippetToEnv,
      commonConfigSnippet,
      envObjToString,
      envStringToObj,
      envValue,
      onEnvChange,
      parseSnippetEnv,
      removeSnippetFromEnv,
      t,
      useCommonConfig,
    ],
  );

  // When env changes, check whether it contains the common config (skipped while the common config itself is updating)
  useEffect(() => {
    if (isUpdatingFromCommonConfig.current || isLoading) {
      return;
    }
    const parsed = parseSnippetEnv(commonConfigSnippet);
    if (parsed.error) return;
    const envObj = envStringToObj(envValue);
    setUseCommonConfig(
      hasEnvCommonConfigSnippet(envObj, parsed.env as Record<string, string>),
    );
  }, [
    envValue,
    commonConfigSnippet,
    envStringToObj,
    hasEnvCommonConfigSnippet,
    isLoading,
    parseSnippetEnv,
  ]);

  // Extract the common config snippet from the editor's current content
  const handleExtract = useCallback(async () => {
    setIsExtracting(true);
    setCommonConfigError("");

    try {
      const extracted = await configApi.extractCommonConfigSnippet("gemini", {
        settingsConfig: JSON.stringify({
          env: envStringToObj(envValue),
        }),
      });

      if (!extracted || extracted === "{}") {
        setCommonConfigError(t("geminiConfig.extractNoCommonConfig"));
        return;
      }

      // Validate JSON
      const parsed = parseSnippetEnv(extracted);
      if (parsed.error) {
        setCommonConfigError(t("geminiConfig.extractedConfigInvalid"));
        return;
      }

      // Update snippet state
      setCommonConfigSnippetState(extracted);

      // Save to the backend
      await configApi.setCommonConfigSnippet("gemini", extracted);
    } catch (error) {
      console.error("Failed to extract Gemini common config:", error);
      setCommonConfigError(
        t("geminiConfig.extractFailed", { error: String(error) }),
      );
    } finally {
      setIsExtracting(false);
    }
  }, [envStringToObj, envValue, parseSnippetEnv, t]);

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
