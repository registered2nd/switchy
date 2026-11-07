import { useState, useCallback, useEffect, useRef } from "react";
import TOML from "smol-toml";
import { useTranslation } from "react-i18next";

/**
 * Validates Codex config.toml syntax
 * Live TOML syntax check with smol-toml (debounced)
 */
export function useCodexTomlValidation() {
  const { t } = useTranslation();
  const [configError, setConfigError] = useState("");
  const debounceTimerRef = useRef<NodeJS.Timeout | null>(null);

  /**
   * Validate TOML syntax
   * @param tomlText - TOML text to validate
   * @returns whether validation passed
   */
  const validateToml = useCallback(
    (tomlText: string): boolean => {
      // An empty string is valid
      if (!tomlText.trim()) {
        setConfigError("");
        return true;
      }

      try {
        TOML.parse(tomlText);
        setConfigError("");
        return true;
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : t("codexConfig.tomlInvalid");
        setConfigError(errorMessage);
        return false;
      }
    },
    [t],
  );

  /**
   * Debounced validation (500 ms delay)
   * @param tomlText - TOML text to validate
   */
  const debouncedValidate = useCallback(
    (tomlText: string) => {
      // Clear the previous timer
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }

      // Start a new timer
      debounceTimerRef.current = setTimeout(() => {
        validateToml(tomlText);
      }, 500);
    },
    [validateToml],
  );

  /**
   * Clear the error message
   */
  const clearError = useCallback(() => {
    setConfigError("");
  }, []);

  // Clean up the timer
  useEffect(() => {
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  return {
    configError,
    validateToml,
    debouncedValidate,
    clearError,
  };
}
