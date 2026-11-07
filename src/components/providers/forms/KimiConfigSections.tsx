import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import JsonEditor from "@/components/JsonEditor";

const useIsDarkMode = () => {
  const [isDarkMode, setIsDarkMode] = useState(false);

  useEffect(() => {
    setIsDarkMode(document.documentElement.classList.contains("dark"));

    const observer = new MutationObserver(() => {
      setIsDarkMode(document.documentElement.classList.contains("dark"));
    });

    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    return () => observer.disconnect();
  }, []);

  return isDarkMode;
};

interface KimiCredentialsSectionProps {
  value: string;
  onChange: (value: string) => void;
  onBlur?: () => void;
  error?: string;
}

/**
 * KimiCredentialsSection - credentials/kimi-code.json editor section
 */
export const KimiCredentialsSection: React.FC<KimiCredentialsSectionProps> = ({
  value,
  onChange,
  onBlur,
  error,
}) => {
  const { t } = useTranslation();
  const isDarkMode = useIsDarkMode();

  const handleChange = (newValue: string) => {
    onChange(newValue);
    if (onBlur) {
      onBlur();
    }
  };

  return (
    <div className="space-y-2">
      <label
        htmlFor="kimiCredentials"
        className="block text-sm font-medium text-foreground"
      >
        {t("kimiConfig.credentialsJson", {
          defaultValue: "Credentials (kimi-code.json)",
        })}
      </label>

      <JsonEditor
        value={value}
        onChange={handleChange}
        placeholder={t("kimiConfig.credentialsJsonPlaceholder", {
          defaultValue: "null",
        })}
        darkMode={isDarkMode}
        rows={6}
        showValidation={true}
        language="json"
      />

      {error && (
        <p className="text-xs text-red-500 dark:text-red-400">{error}</p>
      )}

      {!error && (
        <p className="text-xs text-muted-foreground">
          {t("kimiConfig.credentialsJsonHint", {
            defaultValue:
              "OAuth login written by `kimi login`; null means not logged in or API-key mode",
          })}
        </p>
      )}
    </div>
  );
};

interface KimiConfigSectionProps {
  value: string;
  onChange: (value: string) => void;
  useCommonConfig: boolean;
  onCommonConfigToggle: (checked: boolean) => void;
  onEditCommonConfig: () => void;
  commonConfigError?: string;
  configError?: string;
}

/**
 * KimiConfigSection - Config TOML editor section
 */
export const KimiConfigSection: React.FC<KimiConfigSectionProps> = ({
  value,
  onChange,
  useCommonConfig,
  onCommonConfigToggle,
  onEditCommonConfig,
  commonConfigError,
  configError,
}) => {
  const { t } = useTranslation();
  const isDarkMode = useIsDarkMode();

  // Mirror value prop to local state (same pattern as CodexConfigSection)
  const [localValue, setLocalValue] = useState(value);
  const localValueRef = useRef(value);
  useEffect(() => {
    setLocalValue(value);
    localValueRef.current = value;
  }, [value]);

  const handleLocalChange = useCallback(
    (newValue: string) => {
      if (newValue === localValueRef.current) return;
      localValueRef.current = newValue;
      setLocalValue(newValue);
      onChange(newValue);
    },
    [onChange],
  );

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <label
          htmlFor="kimiConfig"
          className="block text-sm font-medium text-foreground"
        >
          {t("kimiConfig.configToml", { defaultValue: "config.toml (TOML)" })}
        </label>

        <label className="inline-flex items-center gap-2 text-sm text-muted-foreground cursor-pointer">
          <input
            type="checkbox"
            checked={useCommonConfig}
            onChange={(e) => onCommonConfigToggle(e.target.checked)}
            className="w-4 h-4 text-blue-500 bg-white dark:bg-gray-800 border-border-default  rounded focus:ring-blue-500 dark:focus:ring-blue-400 focus:ring-2"
          />
          {t("kimiConfig.writeCommonConfig", {
            defaultValue: "Write Common Config",
          })}
        </label>
      </div>

      <div className="flex items-center justify-end">
        <button
          type="button"
          onClick={onEditCommonConfig}
          className="text-xs text-blue-500 dark:text-blue-400 hover:underline"
        >
          {t("kimiConfig.editCommonConfig", {
            defaultValue: "Edit Common Config",
          })}
        </button>
      </div>

      {commonConfigError && (
        <p className="text-xs text-red-500 dark:text-red-400 text-right">
          {commonConfigError}
        </p>
      )}

      <JsonEditor
        value={localValue}
        onChange={handleLocalChange}
        placeholder=""
        darkMode={isDarkMode}
        rows={8}
        showValidation={false}
        language="javascript"
      />

      {configError && (
        <p className="text-xs text-red-500 dark:text-red-400">{configError}</p>
      )}

      {!configError && (
        <p className="text-xs text-muted-foreground">
          {t("kimiConfig.configTomlHint", {
            defaultValue:
              "Kimi Code config.toml content: default_model, [providers.*], [models.*]",
          })}
        </p>
      )}
    </div>
  );
};
