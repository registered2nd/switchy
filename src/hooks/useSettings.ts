import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { providersApi, settingsApi, type AppId } from "@/lib/api";
import { syncCurrentProvidersLiveSafe } from "@/utils/postChangeSync";
import { useSettingsQuery, useSaveSettingsMutation } from "@/lib/query";
import type { Settings } from "@/types";
import { useSettingsForm, type SettingsFormState } from "./useSettingsForm";
import {
  useDirectorySettings,
  type ResolvedDirectories,
} from "./useDirectorySettings";
import { useSettingsMetadata } from "./useSettingsMetadata";

type Language = "zh" | "en" | "ja";

interface SaveResult {
  requiresRestart: boolean;
}

export interface UseSettingsResult {
  settings: SettingsFormState | null;
  isLoading: boolean;
  isSaving: boolean;
  appConfigDir?: string;
  claudeMirrorDir?: string;
  codexMirrorDir?: string;
  resolvedDirs: ResolvedDirectories;
  requiresRestart: boolean;
  updateSettings: (updates: Partial<SettingsFormState>) => void;
  updateDirectory: (app: AppId, value?: string) => void;
  updateClaudeMirrorDir: (value?: string) => void;
  updateCodexMirrorDir: (value?: string) => void;
  updateAppConfigDir: (value?: string) => void;
  browseDirectory: (app: AppId) => Promise<void>;
  browseClaudeMirrorDir: () => Promise<void>;
  browseCodexMirrorDir: () => Promise<void>;
  browseAppConfigDir: () => Promise<void>;
  resetDirectory: (app: AppId) => Promise<void>;
  resetClaudeMirrorDir: () => Promise<void>;
  resetCodexMirrorDir: () => Promise<void>;
  resetAppConfigDir: () => Promise<void>;
  saveSettings: (
    overrides?: Partial<SettingsFormState>,
    options?: { silent?: boolean },
  ) => Promise<SaveResult | null>;
  autoSaveSettings: (
    updates: Partial<SettingsFormState>,
  ) => Promise<SaveResult | null>;
  resetSettings: () => void;
  acknowledgeRestart: () => void;
}

export type { SettingsFormState, ResolvedDirectories };

const sanitizeDir = (value?: string | null): string | undefined => {
  if (!value) return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
};

/**
 * useSettings - composition layer
 * Handles:
 * - composing useSettingsForm, useDirectorySettings, useSettingsMetadata
 * - saving settings
 * - resetting settings
 */
export function useSettings(): UseSettingsResult {
  const { t } = useTranslation();
  const { data } = useSettingsQuery();
  const saveMutation = useSaveSettingsMutation();

  // 1. Form state
  const {
    settings,
    isLoading: isFormLoading,
    initialLanguage,
    updateSettings,
    resetSettings: resetForm,
    syncLanguage,
  } = useSettingsForm();

  // 2. Directories
  const {
    appConfigDir,
    claudeMirrorDir,
    codexMirrorDir,
    resolvedDirs,
    isLoading: isDirectoryLoading,
    initialAppConfigDir,
    updateDirectory,
    updateClaudeMirrorDir,
    updateCodexMirrorDir,
    updateAppConfigDir,
    browseDirectory,
    browseClaudeMirrorDir,
    browseCodexMirrorDir,
    browseAppConfigDir,
    resetDirectory,
    resetClaudeMirrorDir,
    resetCodexMirrorDir,
    resetAppConfigDir,
    resetAllDirectories,
  } = useDirectorySettings({
    settings,
    onUpdateSettings: updateSettings,
  });

  // 3. Metadata
  const {
    requiresRestart,
    isLoading: isMetadataLoading,
    acknowledgeRestart,
    setRequiresRestart,
  } = useSettingsMetadata();

  // Reset settings
  const resetSettings = useCallback(() => {
    resetForm(data ?? null);
    syncLanguage(initialLanguage);
    resetAllDirectories(
      sanitizeDir(data?.claudeConfigDir),
      sanitizeDir(data?.claudeMirrorConfigDir),
      sanitizeDir(data?.codexConfigDir),
      sanitizeDir(data?.geminiConfigDir),
      sanitizeDir(data?.opencodeConfigDir),
      sanitizeDir(data?.codexMirrorConfigDir),
      sanitizeDir(data?.kimiConfigDir),
    );
    setRequiresRestart(false);
  }, [
    data,
    initialLanguage,
    resetForm,
    syncLanguage,
    resetAllDirectories,
    setRequiresRestart,
  ]);

  // Save settings immediately (live updates on the General tab)
  // Saves the base config plus the separate system API call (launch at login)
  const autoSaveSettings = useCallback(
    async (updates: Partial<SettingsFormState>): Promise<SaveResult | null> => {
      const mergedSettings = settings ? { ...settings, ...updates } : null;
      if (!mergedSettings) return null;

      try {
        const sanitizedClaudeDir = sanitizeDir(mergedSettings.claudeConfigDir);
        const sanitizedClaudeMirrorDir = sanitizeDir(
          mergedSettings.claudeMirrorConfigDir,
        );
        const sanitizedCodexDir = sanitizeDir(mergedSettings.codexConfigDir);
        const sanitizedCodexMirrorDir = sanitizeDir(
          mergedSettings.codexMirrorConfigDir,
        );
        const sanitizedGeminiDir = sanitizeDir(mergedSettings.geminiConfigDir);
        const sanitizedKimiDir = sanitizeDir(mergedSettings.kimiConfigDir);
        const sanitizedOpencodeDir = sanitizeDir(
          mergedSettings.opencodeConfigDir,
        );
        const restSettings = mergedSettings;

        const payload: Settings = {
          ...restSettings,
          claudeConfigDir: sanitizedClaudeDir,
          claudeMirrorConfigDir: sanitizedClaudeMirrorDir,
          codexConfigDir: sanitizedCodexDir,
          codexMirrorConfigDir: sanitizedCodexMirrorDir,
          geminiConfigDir: sanitizedGeminiDir,
          kimiConfigDir: sanitizedKimiDir,
          opencodeConfigDir: sanitizedOpencodeDir,
          language: mergedSettings.language,
        };

        // Save to the config file
        await saveMutation.mutateAsync(payload);

        // Call the system API if the launch-at-login state changed
        if (
          payload.launchOnStartup !== undefined &&
          payload.launchOnStartup !== data?.launchOnStartup
        ) {
          try {
            await settingsApi.setAutoLaunch(payload.launchOnStartup);
          } catch (error) {
            console.error("Failed to update auto-launch:", error);
            toast.error(
              t("settings.autoLaunchFailed", {
                defaultValue: "Failed to set auto-launch",
              }),
            );
          }
        }

        // Persist the language preference
        try {
          if (typeof window !== "undefined" && updates.language) {
            window.localStorage.setItem("language", updates.language);
          }
        } catch (error) {
          console.warn(
            "[useSettings] Failed to persist language preference",
            error,
          );
        }

        // Update the tray menu
        try {
          await providersApi.updateTrayMenu();
        } catch (error) {
          console.warn("[useSettings] Failed to refresh tray menu", error);
        }

        return { requiresRestart: false };
      } catch (error) {
        console.error("[useSettings] Failed to auto-save settings", error);
        toast.error(
          t("notifications.settingsSaveFailed", {
            defaultValue: "Failed to save settings: {{error}}",
            error: (error as Error)?.message ?? String(error),
          }),
        );
        throw error;
      }
    },
    [data, saveMutation, settings, t],
  );

  // Full settings save (manual save on the Advanced tab)
  // Includes every system API call and the full validation flow
  const saveSettings = useCallback(
    async (
      overrides?: Partial<SettingsFormState>,
      options?: { silent?: boolean },
    ): Promise<SaveResult | null> => {
      const mergedSettings = settings ? { ...settings, ...overrides } : null;
      if (!mergedSettings) return null;
      try {
        const sanitizedAppDir = sanitizeDir(appConfigDir);
        const sanitizedClaudeDir = sanitizeDir(mergedSettings.claudeConfigDir);
        const sanitizedClaudeMirrorDir = sanitizeDir(
          mergedSettings.claudeMirrorConfigDir,
        );
        const sanitizedCodexDir = sanitizeDir(mergedSettings.codexConfigDir);
        const sanitizedCodexMirrorDir = sanitizeDir(
          mergedSettings.codexMirrorConfigDir,
        );
        const sanitizedGeminiDir = sanitizeDir(mergedSettings.geminiConfigDir);
        const sanitizedKimiDir = sanitizeDir(mergedSettings.kimiConfigDir);
        const sanitizedOpencodeDir = sanitizeDir(
          mergedSettings.opencodeConfigDir,
        );
        const previousAppDir = initialAppConfigDir;
        const previousClaudeDir = sanitizeDir(data?.claudeConfigDir);
        const previousClaudeMirrorDir = sanitizeDir(
          data?.claudeMirrorConfigDir,
        );
        const previousCodexDir = sanitizeDir(data?.codexConfigDir);
        const previousCodexMirrorDir = sanitizeDir(data?.codexMirrorConfigDir);
        const previousGeminiDir = sanitizeDir(data?.geminiConfigDir);
        const previousKimiDir = sanitizeDir(data?.kimiConfigDir);
        const previousOpencodeDir = sanitizeDir(data?.opencodeConfigDir);
        const restSettings = mergedSettings;

        const payload: Settings = {
          ...restSettings,
          claudeConfigDir: sanitizedClaudeDir,
          claudeMirrorConfigDir: sanitizedClaudeMirrorDir,
          codexConfigDir: sanitizedCodexDir,
          codexMirrorConfigDir: sanitizedCodexMirrorDir,
          geminiConfigDir: sanitizedGeminiDir,
          kimiConfigDir: sanitizedKimiDir,
          opencodeConfigDir: sanitizedOpencodeDir,
          language: mergedSettings.language,
        };

        await saveMutation.mutateAsync(payload);

        await settingsApi.setAppConfigDirOverride(sanitizedAppDir ?? null);

        // Call the system API only when the launch-at-login state actually changed
        if (
          payload.launchOnStartup !== undefined &&
          payload.launchOnStartup !== data?.launchOnStartup
        ) {
          try {
            await settingsApi.setAutoLaunch(payload.launchOnStartup);
          } catch (error) {
            console.error("Failed to update auto-launch:", error);
            toast.error(
              t("settings.autoLaunchFailed", {
                defaultValue: "Failed to set auto-launch",
              }),
            );
          }
        }

        try {
          if (typeof window !== "undefined") {
            window.localStorage.setItem(
              "language",
              payload.language as Language,
            );
          }
        } catch (error) {
          console.warn(
            "[useSettings] Failed to persist language preference",
            error,
          );
        }

        try {
          await providersApi.updateTrayMenu();
        } catch (error) {
          console.warn("[useSettings] Failed to refresh tray menu", error);
        }

        // If the Claude/Codex/Gemini/OpenCode directory override changed, write the current provider back to that app's live config right away
        const claudeDirChanged = sanitizedClaudeDir !== previousClaudeDir;
        const claudeMirrorDirChanged =
          sanitizedClaudeMirrorDir !== previousClaudeMirrorDir;
        const codexDirChanged = sanitizedCodexDir !== previousCodexDir;
        const codexMirrorDirChanged =
          sanitizedCodexMirrorDir !== previousCodexMirrorDir;
        const geminiDirChanged = sanitizedGeminiDir !== previousGeminiDir;
        const kimiDirChanged = sanitizedKimiDir !== previousKimiDir;
        const opencodeDirChanged = sanitizedOpencodeDir !== previousOpencodeDir;
        if (
          claudeDirChanged ||
          claudeMirrorDirChanged ||
          codexDirChanged ||
          codexMirrorDirChanged ||
          geminiDirChanged ||
          kimiDirChanged ||
          opencodeDirChanged
        ) {
          const syncResult = await syncCurrentProvidersLiveSafe();
          if (!syncResult.ok) {
            console.warn(
              "[useSettings] Failed to sync current providers after directory change",
              syncResult.error,
            );
          }
        }

        const appDirChanged = sanitizedAppDir !== (previousAppDir ?? undefined);
        setRequiresRestart(appDirChanged);

        if (!options?.silent) {
          toast.success(
            t("notifications.settingsSaved", {
              defaultValue: "Settings saved",
            }),
            { closeButton: true },
          );
        }

        return { requiresRestart: appDirChanged };
      } catch (error) {
        console.error("[useSettings] Failed to save settings", error);
        toast.error(
          t("notifications.settingsSaveFailed", {
            defaultValue: "Failed to save settings: {{error}}",
            error: (error as Error)?.message ?? String(error),
          }),
        );
        throw error;
      }
    },
    [
      appConfigDir,
      data,
      initialAppConfigDir,
      saveMutation,
      settings,
      setRequiresRestart,
      t,
    ],
  );

  const isLoading = useMemo(
    () => isFormLoading || isDirectoryLoading || isMetadataLoading,
    [isFormLoading, isDirectoryLoading, isMetadataLoading],
  );

  return {
    settings,
    isLoading,
    isSaving: saveMutation.isPending,
    appConfigDir,
    claudeMirrorDir,
    codexMirrorDir,
    resolvedDirs,
    requiresRestart,
    updateSettings,
    updateDirectory,
    updateClaudeMirrorDir,
    updateCodexMirrorDir,
    updateAppConfigDir,
    browseDirectory,
    browseClaudeMirrorDir,
    browseCodexMirrorDir,
    browseAppConfigDir,
    resetDirectory,
    resetClaudeMirrorDir,
    resetCodexMirrorDir,
    resetAppConfigDir,
    saveSettings,
    autoSaveSettings,
    resetSettings,
    acknowledgeRestart,
  };
}
