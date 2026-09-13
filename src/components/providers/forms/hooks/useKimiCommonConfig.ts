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
  /** 仅 Kimi 表单启用；其他应用挂载时不读写通用配置 */
  enabled?: boolean;
}

/**
 * 管理 Kimi 通用配置片段 (TOML 格式)
 * 从 config.json 读取和保存，支持从 localStorage 平滑迁移
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

  // 用于跟踪是否正在通过通用配置更新
  const isUpdatingFromCommonConfig = useRef(false);
  // 用于跟踪新建模式是否已初始化默认勾选
  const hasInitializedNewMode = useRef(false);
  // 用于跟踪编辑模式是否已初始化显式开关/预览
  const hasInitializedEditMode = useRef(false);

  // 当预设变化时，重置初始化标记，使新预设能够重新触发初始化逻辑
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

  // 初始化：从 config.json 加载，支持从 localStorage 迁移
  useEffect(() => {
    if (!enabled) {
      setIsLoading(false);
      return;
    }
    let mounted = true;

    const loadSnippet = async () => {
      try {
        // 使用统一 API 加载
        const snippet = await configApi.getCommonConfigSnippet("kimi");

        if (snippet && snippet.trim()) {
          if (mounted) {
            setCommonConfigSnippetState(snippet);
          }
        } else {
          // 如果 config.json 中没有，尝试从 localStorage 迁移
          if (typeof window !== "undefined") {
            try {
              const legacySnippet =
                window.localStorage.getItem(LEGACY_STORAGE_KEY);
              if (legacySnippet && legacySnippet.trim()) {
                // 迁移到 config.json
                await configApi.setCommonConfigSnippet("kimi", legacySnippet);
                if (mounted) {
                  setCommonConfigSnippetState(legacySnippet);
                }
                // 清理 localStorage
                window.localStorage.removeItem(LEGACY_STORAGE_KEY);
                console.log(
                  "[迁移] Kimi 通用配置已从 localStorage 迁移到 config.json",
                );
              }
            } catch (e) {
              console.warn("[迁移] 从 localStorage 迁移失败:", e);
            }
          }
        }
      } catch (error) {
        console.error("加载 Kimi 通用配置失败:", error);
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

  // 初始化时检查通用配置片段（编辑模式）
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

  // 新建模式：如果通用配置片段存在且有效，默认启用
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

  // 处理通用配置开关
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
            defaultValue: "通用配置片段为空或没有可写入的内容",
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
      // 标记正在通过通用配置更新
      isUpdatingFromCommonConfig.current = true;
      onConfigChange(updatedConfig);
      // 在下一个事件循环中重置标记
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

  // 处理通用配置片段变化
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
          console.error("保存 Kimi 通用配置失败:", error);
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

      // 若当前启用通用配置，需要替换为最新片段
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

        // 标记正在通过通用配置更新，避免触发状态检查
        isUpdatingFromCommonConfig.current = true;
        onConfigChange(addResult.updatedConfig);
        // 在下一个事件循环中重置标记
        setTimeout(() => {
          isUpdatingFromCommonConfig.current = false;
        }, 0);
      }

      setCommonConfigError("");
      setCommonConfigSnippetState(value);
      configApi
        .setCommonConfigSnippet("kimi", value)
        .catch((error: unknown) => {
          console.error("保存 Kimi 通用配置失败:", error);
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

  // 当配置变化时检查是否包含通用配置（但避免在通过通用配置更新时检查）
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

  // 从编辑器当前内容提取通用配置片段
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

      // 更新片段状态
      setCommonConfigSnippetState(extracted);

      // 保存到后端
      await configApi.setCommonConfigSnippet("kimi", extracted);
    } catch (error) {
      console.error("提取 Kimi 通用配置失败:", error);
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
