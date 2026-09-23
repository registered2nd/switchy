import { useState, useCallback, useEffect, useRef } from "react";

interface UseModelStateProps {
  settingsConfig: string;
  onConfigChange: (config: string) => void;
}

/**
 * Parse model values from settings config JSON
 */
function parseModelsFromConfig(settingsConfig: string) {
  try {
    const cfg = settingsConfig ? JSON.parse(settingsConfig) : {};
    const env = cfg?.env || {};
    const model =
      typeof env.ANTHROPIC_MODEL === "string" ? env.ANTHROPIC_MODEL : "";
    const reasoning =
      typeof env.ANTHROPIC_REASONING_MODEL === "string"
        ? env.ANTHROPIC_REASONING_MODEL
        : "";
    const small =
      typeof env.ANTHROPIC_SMALL_FAST_MODEL === "string"
        ? env.ANTHROPIC_SMALL_FAST_MODEL
        : "";
    const haiku =
      typeof env.ANTHROPIC_DEFAULT_HAIKU_MODEL === "string"
        ? env.ANTHROPIC_DEFAULT_HAIKU_MODEL
        : small || model;
    const sonnet =
      typeof env.ANTHROPIC_DEFAULT_SONNET_MODEL === "string"
        ? env.ANTHROPIC_DEFAULT_SONNET_MODEL
        : model || small;
    const opus =
      typeof env.ANTHROPIC_DEFAULT_OPUS_MODEL === "string"
        ? env.ANTHROPIC_DEFAULT_OPUS_MODEL
        : model || small;

    return { model, reasoning, haiku, sonnet, opus };
  } catch {
    return { model: "", reasoning: "", haiku: "", sonnet: "", opus: "" };
  }
}

/**
 * Manages model selection state
 * Supports ANTHROPIC_MODEL, ANTHROPIC_REASONING_MODEL and the per-type default models
 */
export function useModelState({
  settingsConfig,
  onConfigChange,
}: UseModelStateProps) {
  // Initialize state by parsing config directly (fixes edit mode backfill)
  const [claudeModel, setClaudeModel] = useState(
    () => parseModelsFromConfig(settingsConfig).model,
  );
  const [reasoningModel, setReasoningModel] = useState(
    () => parseModelsFromConfig(settingsConfig).reasoning,
  );
  const [defaultHaikuModel, setDefaultHaikuModel] = useState(
    () => parseModelsFromConfig(settingsConfig).haiku,
  );
  const [defaultSonnetModel, setDefaultSonnetModel] = useState(
    () => parseModelsFromConfig(settingsConfig).sonnet,
  );
  const [defaultOpusModel, setDefaultOpusModel] = useState(
    () => parseModelsFromConfig(settingsConfig).opus,
  );

  const isUserEditingRef = useRef(false);
  const lastConfigRef = useRef(settingsConfig);

  // Initial read: use the new keys; if missing, fall back in compatibility order
  // Haiku: DEFAULT_HAIKU || SMALL_FAST || MODEL
  // Sonnet: DEFAULT_SONNET || MODEL || SMALL_FAST
  // Opus: DEFAULT_OPUS || MODEL || SMALL_FAST
  // Sync once, only when settingsConfig changes (form load or preset switch)
  useEffect(() => {
    if (lastConfigRef.current === settingsConfig) {
      return;
    }

    if (isUserEditingRef.current) {
      isUserEditingRef.current = false;
      lastConfigRef.current = settingsConfig;
      return;
    }

    lastConfigRef.current = settingsConfig;

    try {
      const cfg = settingsConfig ? JSON.parse(settingsConfig) : {};
      const env = cfg?.env || {};
      const model =
        typeof env.ANTHROPIC_MODEL === "string" ? env.ANTHROPIC_MODEL : "";
      const reasoning =
        typeof env.ANTHROPIC_REASONING_MODEL === "string"
          ? env.ANTHROPIC_REASONING_MODEL
          : "";
      const small =
        typeof env.ANTHROPIC_SMALL_FAST_MODEL === "string"
          ? env.ANTHROPIC_SMALL_FAST_MODEL
          : "";
      const haiku =
        typeof env.ANTHROPIC_DEFAULT_HAIKU_MODEL === "string"
          ? env.ANTHROPIC_DEFAULT_HAIKU_MODEL
          : small || model;
      const sonnet =
        typeof env.ANTHROPIC_DEFAULT_SONNET_MODEL === "string"
          ? env.ANTHROPIC_DEFAULT_SONNET_MODEL
          : model || small;
      const opus =
        typeof env.ANTHROPIC_DEFAULT_OPUS_MODEL === "string"
          ? env.ANTHROPIC_DEFAULT_OPUS_MODEL
          : model || small;

      setClaudeModel(model || "");
      setReasoningModel(reasoning || "");
      setDefaultHaikuModel(haiku || "");
      setDefaultSonnetModel(sonnet || "");
      setDefaultOpusModel(opus || "");
    } catch {
      // ignore
    }
  }, [settingsConfig]);

  const handleModelChange = useCallback(
    (
      field:
        | "ANTHROPIC_MODEL"
        | "ANTHROPIC_REASONING_MODEL"
        | "ANTHROPIC_DEFAULT_HAIKU_MODEL"
        | "ANTHROPIC_DEFAULT_SONNET_MODEL"
        | "ANTHROPIC_DEFAULT_OPUS_MODEL",
      value: string,
    ) => {
      isUserEditingRef.current = true;

      if (field === "ANTHROPIC_MODEL") setClaudeModel(value);
      if (field === "ANTHROPIC_REASONING_MODEL") setReasoningModel(value);
      if (field === "ANTHROPIC_DEFAULT_HAIKU_MODEL")
        setDefaultHaikuModel(value);
      if (field === "ANTHROPIC_DEFAULT_SONNET_MODEL")
        setDefaultSonnetModel(value);
      if (field === "ANTHROPIC_DEFAULT_OPUS_MODEL") setDefaultOpusModel(value);

      try {
        const currentConfig = settingsConfig
          ? JSON.parse(settingsConfig)
          : { env: {} };
        if (!currentConfig.env) currentConfig.env = {};

        // Write only the new keys; legacy keys are no longer written
        const trimmed = value.trim();
        if (trimmed) {
          currentConfig.env[field] = trimmed;
        } else {
          delete currentConfig.env[field];
        }
        // Remove legacy keys
        delete currentConfig.env["ANTHROPIC_SMALL_FAST_MODEL"];

        onConfigChange(JSON.stringify(currentConfig, null, 2));
      } catch (err) {
        console.error("Failed to update model config:", err);
      }
    },
    [settingsConfig, onConfigChange],
  );

  return {
    claudeModel,
    setClaudeModel,
    reasoningModel,
    setReasoningModel,
    defaultHaikuModel,
    setDefaultHaikuModel,
    defaultSonnetModel,
    setDefaultSonnetModel,
    defaultOpusModel,
    setDefaultOpusModel,
    handleModelChange,
  };
}
