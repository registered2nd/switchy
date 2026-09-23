import { useState, useEffect, useCallback, useMemo } from "react";
import type {
  ProviderPreset,
  TemplateValueConfig,
} from "@/config/claudeProviderPresets";
import type { CodexProviderPreset } from "@/config/codexProviderPresets";
import type { KimiProviderPreset } from "@/config/kimiProviderPresets";
import { applyTemplateValues } from "@/utils/providerConfigUtils";

type TemplatePath = Array<string | number>;
type TemplateValueMap = Record<string, TemplateValueConfig>;

interface PresetEntry {
  id: string;
  preset: ProviderPreset | CodexProviderPreset | KimiProviderPreset;
}

interface UseTemplateValuesProps {
  selectedPresetId: string | null;
  presetEntries: PresetEntry[];
  settingsConfig: string;
  onConfigChange: (config: string) => void;
}

/**
 * Collect the config paths that contain template placeholders
 */
const collectTemplatePaths = (
  source: unknown,
  templateKeys: string[],
  currentPath: TemplatePath = [],
  acc: TemplatePath[] = [],
): TemplatePath[] => {
  if (typeof source === "string") {
    const hasPlaceholder = templateKeys.some((key) =>
      source.includes(`\${${key}}`),
    );
    if (hasPlaceholder) {
      acc.push([...currentPath]);
    }
    return acc;
  }

  if (Array.isArray(source)) {
    source.forEach((item, index) =>
      collectTemplatePaths(item, templateKeys, [...currentPath, index], acc),
    );
    return acc;
  }

  if (source && typeof source === "object") {
    Object.entries(source).forEach(([key, value]) =>
      collectTemplatePaths(value, templateKeys, [...currentPath, key], acc),
    );
  }

  return acc;
};

/**
 * Get a value by path
 */
const getValueAtPath = (source: any, path: TemplatePath) => {
  return path.reduce<any>((acc, key) => {
    if (acc === undefined || acc === null) {
      return undefined;
    }
    return acc[key as keyof typeof acc];
  }, source);
};

/**
 * Set a value by path
 */
const setValueAtPath = (
  target: any,
  path: TemplatePath,
  value: unknown,
): any => {
  if (path.length === 0) {
    return value;
  }

  let current = target;

  for (let i = 0; i < path.length - 1; i++) {
    const key = path[i];
    const nextKey = path[i + 1];
    const isNextIndex = typeof nextKey === "number";

    if (current[key as keyof typeof current] === undefined) {
      current[key as keyof typeof current] = isNextIndex ? [] : {};
    } else {
      const currentValue = current[key as keyof typeof current];
      if (isNextIndex && !Array.isArray(currentValue)) {
        current[key as keyof typeof current] = [];
      } else if (
        !isNextIndex &&
        (typeof currentValue !== "object" || currentValue === null)
      ) {
        current[key as keyof typeof current] = {};
      }
    }

    current = current[key as keyof typeof current];
  }

  const finalKey = path[path.length - 1];
  current[finalKey as keyof typeof current] = value;
  return target;
};

/**
 * Apply template values to the config string (only fields holding a placeholder are updated)
 */
const applyTemplateValuesToConfigString = (
  presetConfig: any,
  currentConfigString: string,
  values: TemplateValueMap,
) => {
  const replacedConfig = applyTemplateValues(presetConfig, values);
  const templateKeys = Object.keys(values);
  if (templateKeys.length === 0) {
    return JSON.stringify(replacedConfig, null, 2);
  }

  const placeholderPaths = collectTemplatePaths(presetConfig, templateKeys);

  try {
    const parsedConfig = currentConfigString.trim()
      ? JSON.parse(currentConfigString)
      : {};
    let targetConfig: any;
    if (Array.isArray(parsedConfig)) {
      targetConfig = [...parsedConfig];
    } else if (parsedConfig && typeof parsedConfig === "object") {
      targetConfig = JSON.parse(JSON.stringify(parsedConfig));
    } else {
      targetConfig = {};
    }

    if (placeholderPaths.length === 0) {
      return JSON.stringify(targetConfig, null, 2);
    }

    let mutatedConfig = targetConfig;

    for (const path of placeholderPaths) {
      const nextValue = getValueAtPath(replacedConfig, path);
      if (path.length === 0) {
        mutatedConfig = nextValue;
      } else {
        setValueAtPath(mutatedConfig, path, nextValue);
      }
    }

    return JSON.stringify(mutatedConfig, null, 2);
  } catch {
    return JSON.stringify(replacedConfig, null, 2);
  }
};

/**
 * Manages template variable state and logic
 */
export function useTemplateValues({
  selectedPresetId,
  presetEntries,
  settingsConfig,
  onConfigChange,
}: UseTemplateValuesProps) {
  const [templateValues, setTemplateValues] = useState<TemplateValueMap>({});

  // Currently selected preset
  const selectedPreset = useMemo(() => {
    if (!selectedPresetId || selectedPresetId === "custom") {
      return null;
    }
    const entry = presetEntries.find((item) => item.id === selectedPresetId);
    // Only handle ProviderPreset (Claude presets)
    if (entry && "settingsConfig" in entry.preset) {
      return entry.preset as ProviderPreset;
    }
    return null;
  }, [selectedPresetId, presetEntries]);

  // Template variable entries
  const templateValueEntries = useMemo(() => {
    if (!selectedPreset?.templateValues) {
      return [];
    }
    return Object.entries(selectedPreset.templateValues) as Array<
      [string, TemplateValueConfig]
    >;
  }, [selectedPreset]);

  // Initialize template values when a preset is selected
  useEffect(() => {
    if (selectedPreset?.templateValues) {
      const initialValues = Object.fromEntries(
        Object.entries(selectedPreset.templateValues).map(([key, config]) => [
          key,
          {
            ...config,
            editorValue: config.editorValue || config.defaultValue || "",
          },
        ]),
      );
      setTemplateValues(initialValues);
    } else {
      setTemplateValues({});
    }
  }, [selectedPreset]);

  // Handle template value changes
  const handleTemplateValueChange = useCallback(
    (key: string, value: string) => {
      if (!selectedPreset?.templateValues) {
        return;
      }

      const config = selectedPreset.templateValues[key];
      if (!config) {
        return;
      }

      setTemplateValues((prev) => {
        const prevEntry = prev[key];
        const nextEntry: TemplateValueConfig = {
          ...config,
          ...(prevEntry ?? {}),
          editorValue: value,
        };
        const nextValues: TemplateValueMap = {
          ...prev,
          [key]: nextEntry,
        };

        // Apply template values to the config
        try {
          const configString = applyTemplateValuesToConfigString(
            selectedPreset.settingsConfig,
            settingsConfig,
            nextValues,
          );
          onConfigChange(configString);
        } catch (err) {
          console.error("Failed to update template values:", err);
        }

        return nextValues;
      });
    },
    [selectedPreset, settingsConfig, onConfigChange],
  );

  // Check that every template value is filled in
  const validateTemplateValues = useCallback((): {
    isValid: boolean;
    missingField?: { key: string; label: string };
  } => {
    if (templateValueEntries.length === 0) {
      return { isValid: true };
    }

    for (const [key, config] of templateValueEntries) {
      const entry = templateValues[key];
      const resolvedValue = (
        entry?.editorValue ??
        entry?.defaultValue ??
        config.defaultValue ??
        ""
      ).trim();
      if (!resolvedValue) {
        return {
          isValid: false,
          missingField: { key, label: config.label },
        };
      }
    }

    return { isValid: true };
  }, [templateValueEntries, templateValues]);

  return {
    templateValues,
    templateValueEntries,
    selectedPreset,
    handleTemplateValueChange,
    validateTemplateValues,
  };
}
