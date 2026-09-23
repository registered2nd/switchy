import { useCallback, useEffect, useMemo, useState } from "react";
import { Loader2, Save } from "lucide-react";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { settingsApi } from "@/lib/api";
import { LanguageSettings } from "@/components/settings/LanguageSettings";
import { ThemeSettings } from "@/components/settings/ThemeSettings";
import { WindowSettings } from "@/components/settings/WindowSettings";
import { AppVisibilitySettings } from "@/components/settings/AppVisibilitySettings";
import { TerminalSettings } from "@/components/settings/TerminalSettings";
import { DirectorySettings } from "@/components/settings/DirectorySettings";
import { ImportExportSection } from "@/components/settings/ImportExportSection";
import { BackupListSection } from "@/components/settings/BackupListSection";
import { AboutSection } from "@/components/settings/AboutSection";
import { PoolTabContent } from "@/components/settings/PoolTabContent";
import { ModelTestConfigPanel } from "@/components/usage/ModelTestConfigPanel";
import { UsageDashboard } from "@/components/usage/UsageDashboard";
import { LogConfigPanel } from "@/components/settings/LogConfigPanel";
import { SessionRecoveryPanel } from "@/components/settings/SessionRecoveryPanel";
import { AuthCenterPanel } from "@/components/settings/AuthCenterPanel";
import { SettingsSection } from "@/components/settings/SettingsSection";
import type { SettingsSection as SettingsSectionId } from "@/components/layout/AppRail";
import { useSettings } from "@/hooks/useSettings";
import { useImportExport } from "@/hooks/useImportExport";
import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportSuccess?: () => void | Promise<void>;
  /** The section shown; the app's sidebar chooses it. */
  activeTab?: SettingsSectionId;
}

export function SettingsPage({
  open,
  onOpenChange,
  onImportSuccess,
  activeTab = "general",
}: SettingsDialogProps) {
  const { t } = useTranslation();
  const {
    settings,
    isLoading,
    isSaving,
    appConfigDir,
    resolvedDirs,
    claudeMirrorDir,
    codexMirrorDir,
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
    requiresRestart,
    acknowledgeRestart,
  } = useSettings();

  const {
    selectedFile,
    status: importStatus,
    errorMessage,
    backupId,
    isImporting,
    selectImportFile,
    importConfig,
    exportConfig,
    clearSelection,
    resetStatus,
  } = useImportExport({ onImportSuccess });

  const [showRestartPrompt, setShowRestartPrompt] = useState(false);

  useEffect(() => {
    if (open) {
      resetStatus();
    }
  }, [open, resetStatus]);

  useEffect(() => {
    if (requiresRestart) {
      setShowRestartPrompt(true);
    }
  }, [requiresRestart]);

  const closeAfterSave = useCallback(() => {
    // Close after a successful save without resetting the language, so one save is enough
    acknowledgeRestart();
    clearSelection();
    resetStatus();
    onOpenChange(false);
  }, [acknowledgeRestart, clearSelection, onOpenChange, resetStatus]);

  const handleSave = useCallback(async () => {
    try {
      const result = await saveSettings(undefined, { silent: false });
      if (!result) return;
      if (result.requiresRestart) {
        setShowRestartPrompt(true);
        return;
      }
      closeAfterSave();
    } catch (error) {
      console.error("[SettingsPage] Failed to save settings", error);
    }
  }, [closeAfterSave, saveSettings]);

  const handleRestartLater = useCallback(() => {
    setShowRestartPrompt(false);
    closeAfterSave();
  }, [closeAfterSave]);

  const handleRestartNow = useCallback(async () => {
    setShowRestartPrompt(false);
    if (import.meta.env.DEV) {
      toast.success(t("settings.devModeRestartHint"), { closeButton: true });
      closeAfterSave();
      return;
    }

    try {
      await settingsApi.restart();
    } catch (error) {
      console.error("[SettingsPage] Failed to restart app", error);
      toast.error(t("settings.restartFailed"));
    } finally {
      closeAfterSave();
    }
  }, [closeAfterSave, t]);

  // General settings save immediately (no save button)
  // autoSaveSettings avoids triggering system APIs by mistake (launch at startup, Claude plugin, etc.)
  const handleAutoSave = useCallback(
    async (updates: Partial<SettingsFormState>) => {
      if (!settings) return;
      updateSettings(updates);
      try {
        await autoSaveSettings(updates);
      } catch (error) {
        console.error("[SettingsPage] Failed to autosave settings", error);
        toast.error(
          t("settings.saveFailedGeneric", {
            defaultValue: "Save failed, please try again",
          }),
        );
      }
    },
    [autoSaveSettings, settings, t, updateSettings],
  );

  const isBusy = useMemo(() => isLoading && !settings, [isLoading, settings]);

  const heading: Record<SettingsSectionId, { title: string; description: string }> = {
    general: {
      title: t("settings.tabGeneral"),
      description: t("settings.page.general", {
        defaultValue: "Language, appearance, the tray and which apps show in the sidebar.",
      }),
    },
    pool: {
      title: t("settings.tabPool"),
      description: t("settings.page.pool", {
        defaultValue:
          "How Switchy routes requests and moves between your accounts.",
      }),
    },
    auth: {
      title: t("settings.tabAuth", { defaultValue: "Sign-in" }),
      description: t("settings.page.auth", {
        defaultValue:
          "Accounts Switchy signs in to for providers that need one, such as GitHub Copilot.",
      }),
    },
    data: {
      title: t("settings.advanced.data.title"),
      description: t("settings.advanced.data.description"),
    },
    advanced: {
      title: t("settings.tabAdvanced"),
      description: t("settings.page.advanced", {
        defaultValue:
          "Where each tool keeps its configuration, provider tests, logging and session repair.",
      }),
    },
    usage: {
      title: t("usage.title"),
      description: t("settings.page.usage", {
        defaultValue:
          "Requests Switchy routed, counted on this machine. Nothing is sent anywhere.",
      }),
    },
    about: {
      title: t("common.about"),
      description: "",
    },
  };

  const renderSection = () => {
    if (!settings) return null;
    switch (activeTab) {
      case "general":
        return (
          <div className="space-y-6">
            <LanguageSettings
              value={settings.language}
              onChange={(lang) => handleAutoSave({ language: lang })}
            />
            <ThemeSettings />
            <AppVisibilitySettings settings={settings} onChange={handleAutoSave} />
            <WindowSettings settings={settings} onChange={handleAutoSave} />
            <TerminalSettings
              value={settings.preferredTerminal}
              onChange={(terminal) =>
                handleAutoSave({ preferredTerminal: terminal })
              }
            />
          </div>
        );
      case "pool":
        return <PoolTabContent settings={settings} onAutoSave={handleAutoSave} />;
      case "auth":
        return <AuthCenterPanel />;
      case "data":
        return (
          <div className="space-y-8">
            <SettingsSection
              title={t("settings.data.backups", { defaultValue: "Backups" })}
              description={t("settings.data.backupsDescription", {
                defaultValue:
                  "Switchy copies its database to ~/.switchy/backups on a schedule. Restore puts an earlier copy back.",
              })}
            >
              <BackupListSection
                backupIntervalHours={settings.backupIntervalHours}
                backupRetainCount={settings.backupRetainCount}
                onSettingsChange={(updates) => handleAutoSave(updates)}
              />
            </SettingsSection>
            <SettingsSection
              title={t("settings.data.move", {
                defaultValue: "Move to another machine",
              })}
              description={t("settings.data.moveDescription", {
                defaultValue:
                  "Export writes the whole database, API keys and Codex logins included, to one file. Import replaces this machine's database with it.",
              })}
            >
              <ImportExportSection
                status={importStatus}
                selectedFile={selectedFile}
                errorMessage={errorMessage}
                backupId={backupId}
                isImporting={isImporting}
                onSelectFile={selectImportFile}
                onImport={importConfig}
                onExport={exportConfig}
                onClear={clearSelection}
              />
            </SettingsSection>
          </div>
        );
      case "advanced":
        return (
          <div className="space-y-8">
            <SettingsSection
              title={t("settings.advanced.configDir.title")}
              description={t("settings.advanced.configDir.description")}
            >
              <DirectorySettings
                appConfigDir={appConfigDir}
                resolvedDirs={resolvedDirs}
                onAppConfigChange={updateAppConfigDir}
                onBrowseAppConfig={browseAppConfigDir}
                onResetAppConfig={resetAppConfigDir}
                claudeDir={settings.claudeConfigDir}
                claudeMirrorDir={claudeMirrorDir}
                codexDir={settings.codexConfigDir}
                codexMirrorDir={codexMirrorDir}
                geminiDir={settings.geminiConfigDir}
                kimiDir={settings.kimiConfigDir}
                opencodeDir={settings.opencodeConfigDir}
                onDirectoryChange={updateDirectory}
                onClaudeMirrorDirChange={updateClaudeMirrorDir}
                onCodexMirrorDirChange={updateCodexMirrorDir}
                onBrowseDirectory={browseDirectory}
                onBrowseClaudeMirrorDir={browseClaudeMirrorDir}
                onBrowseCodexMirrorDir={browseCodexMirrorDir}
                onResetDirectory={resetDirectory}
                onResetClaudeMirrorDir={resetClaudeMirrorDir}
                onResetCodexMirrorDir={resetCodexMirrorDir}
              />
            </SettingsSection>
            <SettingsSection
              title={t("settings.advanced.modelTest.title")}
              description={t("settings.advanced.modelTest.description")}
            >
              <ModelTestConfigPanel />
            </SettingsSection>
            <SettingsSection
              title={t("settings.advanced.logConfig.title")}
              description={t("settings.advanced.logConfig.description")}
            >
              <LogConfigPanel />
            </SettingsSection>
            <SettingsSection
              title={t("settings.advanced.sessionRecovery.title", {
                defaultValue: "Session Recovery",
              })}
              description={t("settings.advanced.sessionRecovery.description", {
                defaultValue:
                  "Repair Claude Code sessions broken by relay-produced thinking blocks (empty signatures / redacted_thinking).",
              })}
            >
              <SessionRecoveryPanel />
            </SettingsSection>
          </div>
        );
      case "usage":
        return <UsageDashboard />;
      case "about":
        return <AboutSection />;
    }
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {isBusy ? (
        <div className="flex flex-1 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <>
          <header
            className="shrink-0 px-8 pb-5 pt-4"
            data-tauri-drag-region
            style={{ WebkitAppRegion: "drag" } as any}
          >
            <h1 className="font-display text-[26px] font-semibold tracking-tight">
              {heading[activeTab].title}
            </h1>
            {heading[activeTab].description && (
              <p className="mt-1 max-w-[62ch] text-[13.5px] text-muted-foreground">
                {heading[activeTab].description}
              </p>
            )}
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-8 pb-10">
            <div className="max-w-[860px]">{renderSection()}</div>
          </div>

          {activeTab === "advanced" && settings && (
            <div className="flex shrink-0 items-center justify-end gap-3 border-t border-border bg-background px-8 py-3">
              <Button onClick={handleSave} disabled={isSaving}>
                {isSaving ? (
                  <span className="inline-flex items-center gap-2">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    {t("settings.saving")}
                  </span>
                ) : (
                  <>
                    <Save className="mr-2 h-4 w-4" />
                    {t("common.save")}
                  </>
                )}
              </Button>
            </div>
          )}
        </>
      )}

      <Dialog
        open={showRestartPrompt}
        onOpenChange={(open) => !open && handleRestartLater()}
      >
        <DialogContent zIndex="alert" className="max-w-md glass border-border">
          <DialogHeader>
            <DialogTitle>{t("settings.restartRequired")}</DialogTitle>
          </DialogHeader>
          <div className="px-6">
            <p className="text-sm text-muted-foreground">
              {t("settings.restartRequiredMessage")}
            </p>
          </div>
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={handleRestartLater}
              className="hover:bg-muted/50"
            >
              {t("settings.restartLater")}
            </Button>
            <Button
              onClick={handleRestartNow}
              className="bg-primary hover:bg-primary/90"
            >
              {t("settings.restartNow")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
