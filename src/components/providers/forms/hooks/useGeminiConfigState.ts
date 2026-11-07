import { useState, useCallback, useEffect } from "react";

interface UseGeminiConfigStateProps {
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
}

/**
 * Manages Gemini config state
 * Gemini config has two parts: env (environment variables) and config (extension settings JSON)
 */
export function useGeminiConfigState({
  initialData,
}: UseGeminiConfigStateProps) {
  const [geminiEnv, setGeminiEnvState] = useState("");
  const [geminiConfig, setGeminiConfigState] = useState("");
  const [geminiApiKey, setGeminiApiKey] = useState("");
  const [geminiBaseUrl, setGeminiBaseUrl] = useState("");
  const [geminiModel, setGeminiModel] = useState("");
  const [envError, setEnvError] = useState("");
  const [configError, setConfigError] = useState("");

  // Convert a JSON env object to a .env string
  // Keep every variable; known keys come first
  const envObjToString = useCallback(
    (envObj: Record<string, unknown>): string => {
      const priorityKeys = [
        "GOOGLE_GEMINI_BASE_URL",
        "GEMINI_API_KEY",
        "GEMINI_MODEL",
      ];
      const lines: string[] = [];
      const addedKeys = new Set<string>();

      // Known keys first, in order
      for (const key of priorityKeys) {
        if (typeof envObj[key] === "string" && envObj[key]) {
          lines.push(`${key}=${envObj[key]}`);
          addedKeys.add(key);
        }
      }

      // Then any other keys (keeps variables the user added)
      for (const [key, value] of Object.entries(envObj)) {
        if (!addedKeys.has(key) && typeof value === "string") {
          lines.push(`${key}=${value}`);
        }
      }

      return lines.join("\n");
    },
    [],
  );

  // Convert a .env string to a JSON env object
  const envStringToObj = useCallback(
    (envString: string): Record<string, string> => {
      const env: Record<string, string> = {};
      const lines = envString.split("\n");
      lines.forEach((line) => {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith("#")) return;
        const equalIndex = trimmed.indexOf("=");
        if (equalIndex > 0) {
          const key = trimmed.substring(0, equalIndex).trim();
          const value = trimmed.substring(equalIndex + 1).trim();
          env[key] = value;
        }
      });
      return env;
    },
    [],
  );

  // Initialize Gemini config (edit mode)
  useEffect(() => {
    if (!initialData) return;

    const config = initialData.settingsConfig;
    if (typeof config === "object" && config !== null) {
      // Set env
      const env = (config as any).env || {};
      setGeminiEnvState(envObjToString(env));

      // Set config
      const configObj = (config as any).config || {};
      setGeminiConfigState(JSON.stringify(configObj, null, 2));

      // Extract the API key, Base URL and model
      if (typeof env.GEMINI_API_KEY === "string") {
        setGeminiApiKey(env.GEMINI_API_KEY);
      }
      if (typeof env.GOOGLE_GEMINI_BASE_URL === "string") {
        setGeminiBaseUrl(env.GOOGLE_GEMINI_BASE_URL);
      }
      if (typeof env.GEMINI_MODEL === "string") {
        setGeminiModel(env.GEMINI_MODEL);
      }
    }
  }, [initialData, envObjToString]);

  // Extract the API key, Base URL and model from geminiEnv and sync them
  useEffect(() => {
    const envObj = envStringToObj(geminiEnv);
    const extractedKey = envObj.GEMINI_API_KEY || "";
    const extractedBaseUrl = envObj.GOOGLE_GEMINI_BASE_URL || "";
    const extractedModel = envObj.GEMINI_MODEL || "";

    if (extractedKey !== geminiApiKey) {
      setGeminiApiKey(extractedKey);
    }
    if (extractedBaseUrl !== geminiBaseUrl) {
      setGeminiBaseUrl(extractedBaseUrl);
    }
    if (extractedModel !== geminiModel) {
      setGeminiModel(extractedModel);
    }
  }, [geminiEnv, envStringToObj, geminiApiKey, geminiBaseUrl, geminiModel]);

  // Validate Gemini config JSON
  const validateGeminiConfig = useCallback((value: string): string => {
    if (!value.trim()) return ""; // empty is allowed
    try {
      const parsed = JSON.parse(value);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return "";
      }
      return "Config must be a JSON object";
    } catch {
      return "Invalid JSON format";
    }
  }, []);

  // Set env
  const setGeminiEnv = useCallback((value: string) => {
    setGeminiEnvState(value);
    // .env is loose; no strict validation
    setEnvError("");
  }, []);

  // Set config (accepts an updater function)
  const setGeminiConfig = useCallback(
    (value: string | ((prev: string) => string)) => {
      const newValue =
        typeof value === "function" ? value(geminiConfig) : value;
      setGeminiConfigState(newValue);
      setConfigError(validateGeminiConfig(newValue));
    },
    [geminiConfig, validateGeminiConfig],
  );

  // Handle Gemini API key input and write it back to env
  const handleGeminiApiKeyChange = useCallback(
    (key: string) => {
      const trimmed = key.trim();
      setGeminiApiKey(trimmed);

      const envObj = envStringToObj(geminiEnv);
      envObj.GEMINI_API_KEY = trimmed;
      const newEnv = envObjToString(envObj);
      setGeminiEnv(newEnv);
    },
    [geminiEnv, envStringToObj, envObjToString, setGeminiEnv],
  );

  // Handle Gemini Base URL changes
  const handleGeminiBaseUrlChange = useCallback(
    (url: string) => {
      const sanitized = url.trim().replace(/\/+$/, "");
      setGeminiBaseUrl(sanitized);

      const envObj = envStringToObj(geminiEnv);
      envObj.GOOGLE_GEMINI_BASE_URL = sanitized;
      const newEnv = envObjToString(envObj);
      setGeminiEnv(newEnv);
    },
    [geminiEnv, envStringToObj, envObjToString, setGeminiEnv],
  );

  // Handle Gemini model changes
  const handleGeminiModelChange = useCallback(
    (model: string) => {
      const trimmed = model.trim();
      setGeminiModel(trimmed);

      const envObj = envStringToObj(geminiEnv);
      envObj.GEMINI_MODEL = trimmed;
      const newEnv = envObjToString(envObj);
      setGeminiEnv(newEnv);
    },
    [geminiEnv, envStringToObj, envObjToString, setGeminiEnv],
  );

  // Handle env changes
  const handleGeminiEnvChange = useCallback(
    (value: string) => {
      setGeminiEnv(value);
    },
    [setGeminiEnv],
  );

  // Handle config changes
  const handleGeminiConfigChange = useCallback(
    (value: string) => {
      setGeminiConfig(value);
    },
    [setGeminiConfig],
  );

  // Reset config (on preset switch)
  const resetGeminiConfig = useCallback(
    (env: Record<string, unknown>, config: Record<string, unknown>) => {
      const envString = envObjToString(env);
      const configString = JSON.stringify(config, null, 2);

      setGeminiEnv(envString);
      setGeminiConfig(configString);

      // Extract the API key, Base URL and model
      if (typeof env.GEMINI_API_KEY === "string") {
        setGeminiApiKey(env.GEMINI_API_KEY);
      } else {
        setGeminiApiKey("");
      }

      if (typeof env.GOOGLE_GEMINI_BASE_URL === "string") {
        setGeminiBaseUrl(env.GOOGLE_GEMINI_BASE_URL);
      } else {
        setGeminiBaseUrl("");
      }

      if (typeof env.GEMINI_MODEL === "string") {
        setGeminiModel(env.GEMINI_MODEL);
      } else {
        setGeminiModel("");
      }
    },
    [envObjToString, setGeminiEnv, setGeminiConfig],
  );

  return {
    geminiEnv,
    geminiConfig,
    geminiApiKey,
    geminiBaseUrl,
    geminiModel,
    envError,
    configError,
    setGeminiEnv,
    setGeminiConfig,
    handleGeminiApiKeyChange,
    handleGeminiBaseUrlChange,
    handleGeminiModelChange,
    handleGeminiEnvChange,
    handleGeminiConfigChange,
    resetGeminiConfig,
    envStringToObj,
    envObjToString,
  };
}
