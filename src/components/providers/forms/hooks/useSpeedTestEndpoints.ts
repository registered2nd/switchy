import { useMemo } from "react";
import type { AppId } from "@/lib/api";
import type { ProviderPreset } from "@/config/claudeProviderPresets";
import type { CodexProviderPreset } from "@/config/codexProviderPresets";
import type { ProviderMeta, EndpointCandidate } from "@/types";
import {
  extractCodexBaseUrl,
  extractKimiBaseUrl,
} from "@/utils/providerConfigUtils";
import type { KimiProviderPreset } from "@/config/kimiProviderPresets";

type PresetEntry = {
  id: string;
  preset: ProviderPreset | CodexProviderPreset | KimiProviderPreset;
};

interface UseSpeedTestEndpointsProps {
  appId: AppId;
  selectedPresetId: string | null;
  presetEntries: PresetEntry[];
  baseUrl: string;
  codexBaseUrl: string;
  kimiBaseUrl?: string;
  initialData?: {
    settingsConfig?: Record<string, unknown>;
    meta?: ProviderMeta;
  };
}

/**
 * Collects the initial endpoint list for the endpoint speed-test dialog
 *
 * Sources:
 * 1. The currently selected Base URL
 * 2. The initial-data URL in edit mode
 * 3. endpointCandidates from the preset
 *
 * Note: saved custom endpoints are loaded by EndpointSpeedTest through the getCustomEndpoints API,
 * not read here, to avoid importing them twice.
 */
export function useSpeedTestEndpoints({
  appId,
  selectedPresetId,
  presetEntries,
  baseUrl,
  codexBaseUrl,
  kimiBaseUrl = "",
  initialData,
}: UseSpeedTestEndpointsProps) {
  const claudeEndpoints = useMemo<EndpointCandidate[]>(() => {
    // Reuse this branch for Claude and Gemini (non-Codex)
    if (appId !== "claude" && appId !== "gemini") return [];

    const map = new Map<string, EndpointCandidate>();
    // Candidates are marked isCustom: false, meaning they come from the preset or config
    // Saved custom endpoints are loaded through the API in EndpointSpeedTest
    const add = (url?: string, isCustom = false) => {
      if (!url) return;
      const sanitized = url.trim().replace(/\/+$/, "");
      if (!sanitized || map.has(sanitized)) return;
      map.set(sanitized, { url: sanitized, isCustom });
    };

    // 1. Current Base URL
    if (baseUrl) {
      add(baseUrl);
    }

    // 2. Edit mode: URL from the initial data
    if (initialData && typeof initialData.settingsConfig === "object") {
      const configEnv = initialData.settingsConfig as {
        env?: { ANTHROPIC_BASE_URL?: string; GOOGLE_GEMINI_BASE_URL?: string };
      };
      const envUrls = [
        configEnv.env?.ANTHROPIC_BASE_URL,
        configEnv.env?.GOOGLE_GEMINI_BASE_URL,
      ];
      envUrls.forEach((u) => {
        if (typeof u === "string") add(u);
      });
    }

    // 3. endpointCandidates from the preset
    if (selectedPresetId && selectedPresetId !== "custom") {
      const entry = presetEntries.find((item) => item.id === selectedPresetId);
      if (entry) {
        const preset = entry.preset as ProviderPreset & {
          settingsConfig?: { env?: { GOOGLE_GEMINI_BASE_URL?: string } };
          endpointCandidates?: string[];
        };
        // Add the preset's own baseUrl (Claude/Gemini)
        const presetEnv = preset.settingsConfig as {
          env?: {
            ANTHROPIC_BASE_URL?: string;
            GOOGLE_GEMINI_BASE_URL?: string;
          };
        };
        const presetUrls = [
          presetEnv?.env?.ANTHROPIC_BASE_URL,
          presetEnv?.env?.GOOGLE_GEMINI_BASE_URL,
        ];
        presetUrls.forEach((u) => add(u));
        // Add the preset's candidate endpoints
        if (preset.endpointCandidates) {
          preset.endpointCandidates.forEach((url) => add(url));
        }
      }
    }

    return Array.from(map.values());
  }, [appId, baseUrl, initialData, selectedPresetId, presetEntries]);

  const codexEndpoints = useMemo<EndpointCandidate[]>(() => {
    if (appId !== "codex") return [];

    const map = new Map<string, EndpointCandidate>();
    // Candidates are marked isCustom: false, meaning they come from the preset or config
    // Saved custom endpoints are loaded through the API in EndpointSpeedTest
    const add = (url?: string, isCustom = false) => {
      if (!url) return;
      const sanitized = url.trim().replace(/\/+$/, "");
      if (!sanitized || map.has(sanitized)) return;
      map.set(sanitized, { url: sanitized, isCustom });
    };

    // 1. Current Codex Base URL
    if (codexBaseUrl) {
      add(codexBaseUrl);
    }

    // 2. Edit mode: URL from the initial data
    const initialCodexConfig = initialData?.settingsConfig as
      | {
          config?: string;
        }
      | undefined;
    const configStr = initialCodexConfig?.config ?? "";
    const extractedBaseUrl = extractCodexBaseUrl(configStr);
    if (extractedBaseUrl) {
      add(extractedBaseUrl);
    }

    // 3. endpointCandidates from the preset
    if (selectedPresetId && selectedPresetId !== "custom") {
      const entry = presetEntries.find((item) => item.id === selectedPresetId);
      if (entry) {
        const preset = entry.preset as CodexProviderPreset;
        // Add the preset's own baseUrl
        const presetConfig = preset.config || "";
        const presetBaseUrl = extractCodexBaseUrl(presetConfig);
        if (presetBaseUrl) {
          add(presetBaseUrl);
        }
        // Add the preset's candidate endpoints
        if (preset.endpointCandidates) {
          preset.endpointCandidates.forEach((url) => add(url));
        }
      }
    }

    return Array.from(map.values());
  }, [appId, codexBaseUrl, initialData, selectedPresetId, presetEntries]);

  const kimiEndpoints = useMemo<EndpointCandidate[]>(() => {
    if (appId !== "kimi") return [];

    const map = new Map<string, EndpointCandidate>();
    const add = (url?: string, isCustom = false) => {
      if (!url) return;
      const sanitized = url.trim().replace(/\/+$/, "");
      if (!sanitized || map.has(sanitized)) return;
      map.set(sanitized, { url: sanitized, isCustom });
    };

    // 1. Current Kimi Base URL
    if (kimiBaseUrl) {
      add(kimiBaseUrl);
    }

    // 2. Edit mode: URL from the initial data
    const initialKimiConfig = initialData?.settingsConfig as
      | {
          config?: string;
        }
      | undefined;
    const extractedBaseUrl = extractKimiBaseUrl(
      initialKimiConfig?.config ?? "",
    );
    if (extractedBaseUrl) {
      add(extractedBaseUrl);
    }

    // 3. endpointCandidates from the preset
    if (selectedPresetId && selectedPresetId !== "custom") {
      const entry = presetEntries.find((item) => item.id === selectedPresetId);
      if (entry) {
        const preset = entry.preset as KimiProviderPreset;
        const presetBaseUrl = extractKimiBaseUrl(preset.config || "");
        if (presetBaseUrl) {
          add(presetBaseUrl);
        }
        if (preset.endpointCandidates) {
          preset.endpointCandidates.forEach((url) => add(url));
        }
      }
    }

    return Array.from(map.values());
  }, [appId, kimiBaseUrl, initialData, selectedPresetId, presetEntries]);

  if (appId === "codex") return codexEndpoints;
  if (appId === "kimi") return kimiEndpoints;
  return claudeEndpoints;
}
