import { useState, useCallback, useEffect, useRef } from "react";
import {
  extractKimiBaseUrl,
  setKimiBaseUrl as setKimiBaseUrlInConfig,
  extractKimiModelName,
  setKimiModelName as setKimiModelNameInConfig,
  extractKimiApiKey,
  setKimiApiKey as setKimiApiKeyInConfig,
} from "@/utils/providerConfigUtils";
import { normalizeTomlText } from "@/utils/textNormalization";

interface UseKimiConfigStateProps {
  initialData?: {
    settingsConfig?: Record<string, unknown>;
  };
}

/**
 * 管理 Kimi 配置状态
 * Kimi 配置包含两部分：credentials/kimi-code.json (JSON) 和 config.toml (TOML 字符串)。
 * API Key / 请求地址 / 模型名都存放在 config.toml 里，与 Codex 把 key 放在 auth.json 不同。
 */
export function useKimiConfigState({ initialData }: UseKimiConfigStateProps) {
  const [kimiCredentials, setKimiCredentialsState] = useState("");
  const [kimiConfig, setKimiConfigState] = useState("");
  const [kimiApiKey, setKimiApiKey] = useState("");
  const [kimiBaseUrl, setKimiBaseUrl] = useState("");
  const [kimiModelName, setKimiModelName] = useState("");
  const [kimiCredentialsError, setKimiCredentialsError] = useState("");

  const isUpdatingKimiBaseUrlRef = useRef(false);
  const isUpdatingKimiModelNameRef = useRef(false);
  const isUpdatingKimiApiKeyRef = useRef(false);

  const syncFieldsFromConfig = useCallback((configStr: string) => {
    setKimiBaseUrl(extractKimiBaseUrl(configStr) ?? "");
    setKimiModelName(extractKimiModelName(configStr) ?? "");
    setKimiApiKey(extractKimiApiKey(configStr) ?? "");
  }, []);

  // 初始化 Kimi 配置（编辑模式）
  useEffect(() => {
    if (!initialData) return;

    const config = initialData.settingsConfig;
    if (typeof config === "object" && config !== null) {
      const credentials = (config as any).credentials;
      setKimiCredentialsState(
        credentials === null || credentials === undefined
          ? "null"
          : JSON.stringify(credentials, null, 2),
      );

      const configStr =
        typeof (config as any).config === "string"
          ? (config as any).config
          : "";
      setKimiConfigState(configStr);
      syncFieldsFromConfig(configStr);
    }
  }, [initialData, syncFieldsFromConfig]);

  // 与 TOML 配置保持基础 URL 同步
  useEffect(() => {
    if (isUpdatingKimiBaseUrlRef.current) return;
    const extracted = extractKimiBaseUrl(kimiConfig) || "";
    setKimiBaseUrl((prev) => (prev === extracted ? prev : extracted));
  }, [kimiConfig]);

  // 与 TOML 配置保持模型名称同步
  useEffect(() => {
    if (isUpdatingKimiModelNameRef.current) return;
    const extracted = extractKimiModelName(kimiConfig) || "";
    setKimiModelName((prev) => (prev === extracted ? prev : extracted));
  }, [kimiConfig]);

  // 与 TOML 配置保持 API Key 同步
  useEffect(() => {
    if (isUpdatingKimiApiKeyRef.current) return;
    const extracted = extractKimiApiKey(kimiConfig) || "";
    setKimiApiKey((prev) => (prev === extracted ? prev : extracted));
  }, [kimiConfig]);

  // 验证 credentials JSON（允许 null / 空）
  const validateKimiCredentials = useCallback((value: string): string => {
    const trimmed = value.trim();
    if (!trimmed || trimmed === "null") return "";
    try {
      const parsed = JSON.parse(trimmed);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        return "Credentials JSON must be an object or null";
      }
      return "";
    } catch {
      return "Invalid JSON format";
    }
  }, []);

  const setKimiCredentials = useCallback(
    (value: string) => {
      setKimiCredentialsState(value);
      setKimiCredentialsError(validateKimiCredentials(value));
    },
    [validateKimiCredentials],
  );

  /** credentials 文本 → 对象（空 / "null" / 非法 JSON 都视为 null） */
  const parseKimiCredentials = useCallback(
    (value: string): Record<string, unknown> | null => {
      const trimmed = value.trim();
      if (!trimmed || trimmed === "null") return null;
      try {
        const parsed = JSON.parse(trimmed);
        return parsed && typeof parsed === "object" && !Array.isArray(parsed)
          ? parsed
          : null;
      } catch {
        return null;
      }
    },
    [],
  );

  const setKimiConfig = useCallback(
    (value: string | ((prev: string) => string)) => {
      setKimiConfigState((prev) =>
        typeof value === "function"
          ? (value as (input: string) => string)(prev)
          : value,
      );
    },
    [],
  );

  const handleKimiApiKeyChange = useCallback(
    (key: string) => {
      const trimmed = key.trim();
      setKimiApiKey(trimmed);

      isUpdatingKimiApiKeyRef.current = true;
      setKimiConfig((prev) => setKimiApiKeyInConfig(prev, trimmed));
      setTimeout(() => {
        isUpdatingKimiApiKeyRef.current = false;
      }, 0);
    },
    [setKimiConfig],
  );

  const handleKimiBaseUrlChange = useCallback(
    (url: string) => {
      const sanitized = url.trim();
      setKimiBaseUrl(sanitized);

      isUpdatingKimiBaseUrlRef.current = true;
      setKimiConfig((prev) => setKimiBaseUrlInConfig(prev, sanitized));
      setTimeout(() => {
        isUpdatingKimiBaseUrlRef.current = false;
      }, 0);
    },
    [setKimiConfig],
  );

  const handleKimiModelNameChange = useCallback(
    (modelName: string) => {
      const trimmed = modelName.trim();
      setKimiModelName(trimmed);

      isUpdatingKimiModelNameRef.current = true;
      setKimiConfig((prev) => setKimiModelNameInConfig(prev, trimmed));
      setTimeout(() => {
        isUpdatingKimiModelNameRef.current = false;
      }, 0);
    },
    [setKimiConfig],
  );

  // 处理 config 变化（同步 Base URL / Model Name / API Key）
  const handleKimiConfigChange = useCallback(
    (value: string) => {
      const normalized = normalizeTomlText(value);
      setKimiConfig(normalized);

      if (!isUpdatingKimiBaseUrlRef.current) {
        const extracted = extractKimiBaseUrl(normalized) || "";
        if (extracted !== kimiBaseUrl) setKimiBaseUrl(extracted);
      }
      if (!isUpdatingKimiModelNameRef.current) {
        const extractedModel = extractKimiModelName(normalized) || "";
        if (extractedModel !== kimiModelName) setKimiModelName(extractedModel);
      }
      if (!isUpdatingKimiApiKeyRef.current) {
        const extractedKey = extractKimiApiKey(normalized) || "";
        if (extractedKey !== kimiApiKey) setKimiApiKey(extractedKey);
      }
    },
    [setKimiConfig, kimiBaseUrl, kimiModelName, kimiApiKey],
  );

  // 重置配置（用于预设切换）
  const resetKimiConfig = useCallback(
    (credentials: Record<string, unknown> | null, config: string) => {
      setKimiCredentials(
        credentials === null ? "null" : JSON.stringify(credentials, null, 2),
      );
      setKimiConfig(config);
      syncFieldsFromConfig(config);
    },
    [setKimiCredentials, setKimiConfig, syncFieldsFromConfig],
  );

  return {
    kimiCredentials,
    kimiConfig,
    kimiApiKey,
    kimiBaseUrl,
    kimiModelName,
    kimiCredentialsError,
    setKimiCredentials,
    setKimiConfig,
    parseKimiCredentials,
    handleKimiApiKeyChange,
    handleKimiBaseUrlChange,
    handleKimiModelNameChange,
    handleKimiConfigChange,
    resetKimiConfig,
    validateKimiCredentials,
  };
}
