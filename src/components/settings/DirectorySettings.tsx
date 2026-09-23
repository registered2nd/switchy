import { useMemo } from "react";
import { FolderSearch, Undo2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import type { ResolvedDirectories } from "@/hooks/useSettings";

interface DirectorySettingsProps {
  appConfigDir?: string;
  resolvedDirs: ResolvedDirectories;
  onAppConfigChange: (value?: string) => void;
  onBrowseAppConfig: () => Promise<void>;
  onResetAppConfig: () => Promise<void>;
  claudeDir?: string;
  claudeMirrorDir?: string;
  codexDir?: string;
  codexMirrorDir?: string;
  geminiDir?: string;
  kimiDir?: string;
  opencodeDir?: string;
  onDirectoryChange: (app: AppId, value?: string) => void;
  onClaudeMirrorDirChange: (value?: string) => void;
  onCodexMirrorDirChange: (value?: string) => void;
  onBrowseDirectory: (app: AppId) => Promise<void>;
  onBrowseClaudeMirrorDir: () => Promise<void>;
  onBrowseCodexMirrorDir: () => Promise<void>;
  onResetDirectory: (app: AppId) => Promise<void>;
  onResetClaudeMirrorDir: () => Promise<void>;
  onResetCodexMirrorDir: () => Promise<void>;
}

export function DirectorySettings({
  appConfigDir,
  resolvedDirs,
  onAppConfigChange,
  onBrowseAppConfig,
  onResetAppConfig,
  claudeDir,
  claudeMirrorDir,
  codexDir,
  codexMirrorDir,
  geminiDir,
  kimiDir,
  opencodeDir,
  onDirectoryChange,
  onClaudeMirrorDirChange,
  onCodexMirrorDirChange,
  onBrowseDirectory,
  onBrowseClaudeMirrorDir,
  onBrowseCodexMirrorDir,
  onResetDirectory,
  onResetClaudeMirrorDir,
  onResetCodexMirrorDir,
}: DirectorySettingsProps) {
  const { t } = useTranslation();

  return (
    <div className="space-y-6">
      {/* Switchy config directory (own section) */}
      <section className="space-y-4">
        <header className="space-y-1">
          <h3 className="text-sm font-medium">{t("settings.appConfigDir")}</h3>
          <p className="text-xs text-muted-foreground">
            {t("settings.appConfigDirDescription")}
          </p>
        </header>

        <div className="flex items-center gap-2">
          <Input
            value={appConfigDir ?? resolvedDirs.appConfig ?? ""}
            placeholder={t("settings.browsePlaceholderApp")}
            className="text-xs"
            onChange={(event) => onAppConfigChange(event.target.value)}
          />
          <Button
            type="button"
            variant="outline"
            size="icon"
            onClick={onBrowseAppConfig}
            title={t("settings.browseDirectory")}
          >
            <FolderSearch className="h-4 w-4" />
          </Button>
          <Button
            type="button"
            variant="outline"
            size="icon"
            onClick={onResetAppConfig}
            title={t("settings.resetDefault")}
          >
            <Undo2 className="h-4 w-4" />
          </Button>
        </div>
      </section>

      {/* Claude/Codex config directories (own section) */}
      <section className="space-y-4">
        <header className="space-y-1">
          <h3 className="text-sm font-medium">
            {t("settings.configDirectoryOverride")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t("settings.configDirectoryDescription")}
          </p>
        </header>

        <DirectoryInput
          label={t("settings.claudeConfigDir")}
          description={undefined}
          value={claudeDir}
          resolvedValue={resolvedDirs.claude}
          placeholder={t("settings.browsePlaceholderClaude")}
          onChange={(val) => onDirectoryChange("claude", val)}
          onBrowse={() => onBrowseDirectory("claude")}
          onReset={() => onResetDirectory("claude")}
        />

        <DirectoryInput
          label={t("settings.claudeMirrorConfigDir", {
            defaultValue: "Claude Code Mirror Directory",
          })}
          description={t("settings.claudeMirrorConfigDirDescription", {
            defaultValue:
              "Optional second Claude configuration directory to keep in sync, such as WSL ~/.claude.",
          })}
          value={claudeMirrorDir}
          resolvedValue=""
          placeholder={t("settings.browsePlaceholderClaude")}
          onChange={onClaudeMirrorDirChange}
          onBrowse={onBrowseClaudeMirrorDir}
          onReset={onResetClaudeMirrorDir}
        />

        <DirectoryInput
          label={t("settings.codexConfigDir")}
          description={undefined}
          value={codexDir}
          resolvedValue={resolvedDirs.codex}
          placeholder={t("settings.browsePlaceholderCodex")}
          onChange={(val) => onDirectoryChange("codex", val)}
          onBrowse={() => onBrowseDirectory("codex")}
          onReset={() => onResetDirectory("codex")}
        />

        <DirectoryInput
          label={t("settings.codexMirrorConfigDir", {
            defaultValue: "Codex Mirror Directory",
          })}
          description={t("settings.codexMirrorConfigDirDescription", {
            defaultValue:
              "Optional second Codex configuration directory to keep in sync, such as WSL ~/.codex.",
          })}
          value={codexMirrorDir}
          resolvedValue=""
          placeholder={t("settings.browsePlaceholderCodex")}
          onChange={onCodexMirrorDirChange}
          onBrowse={onBrowseCodexMirrorDir}
          onReset={onResetCodexMirrorDir}
        />

        <DirectoryInput
          label={t("settings.geminiConfigDir")}
          description={undefined}
          value={geminiDir}
          resolvedValue={resolvedDirs.gemini}
          placeholder={t("settings.browsePlaceholderGemini")}
          onChange={(val) => onDirectoryChange("gemini", val)}
          onBrowse={() => onBrowseDirectory("gemini")}
          onReset={() => onResetDirectory("gemini")}
        />

        <DirectoryInput
          label={t("settings.kimiConfigDir", {
            defaultValue: "Kimi Code Configuration Directory",
          })}
          description={undefined}
          value={kimiDir}
          resolvedValue={resolvedDirs.kimi}
          placeholder={t("settings.browsePlaceholderKimi", {
            defaultValue: "e.g., /home/<your-username>/.kimi-code",
          })}
          onChange={(val) => onDirectoryChange("kimi", val)}
          onBrowse={() => onBrowseDirectory("kimi")}
          onReset={() => onResetDirectory("kimi")}
        />

        <DirectoryInput
          label={t("settings.opencodeConfigDir")}
          description={undefined}
          value={opencodeDir}
          resolvedValue={resolvedDirs.opencode}
          placeholder={t("settings.browsePlaceholderOpencode")}
          onChange={(val) => onDirectoryChange("opencode", val)}
          onBrowse={() => onBrowseDirectory("opencode")}
          onReset={() => onResetDirectory("opencode")}
        />
      </section>
    </div>
  );
}

interface DirectoryInputProps {
  label: string;
  description?: string;
  value?: string;
  resolvedValue: string;
  placeholder?: string;
  onChange: (value?: string) => void;
  onBrowse: () => Promise<void>;
  onReset: () => Promise<void>;
}

function DirectoryInput({
  label,
  description,
  value,
  resolvedValue,
  placeholder,
  onChange,
  onBrowse,
  onReset,
}: DirectoryInputProps) {
  const { t } = useTranslation();
  const displayValue = useMemo(
    () => value ?? resolvedValue ?? "",
    [value, resolvedValue],
  );

  return (
    <div className="space-y-1.5">
      <div className="space-y-1">
        <p className="text-xs font-medium text-foreground">{label}</p>
        {description ? (
          <p className="text-xs text-muted-foreground">{description}</p>
        ) : null}
      </div>
      <div className="flex items-center gap-2">
        <Input
          value={displayValue}
          placeholder={placeholder}
          className="text-xs"
          onChange={(event) => onChange(event.target.value)}
        />
        <Button
          type="button"
          variant="outline"
          size="icon"
          onClick={onBrowse}
          title={t("settings.browseDirectory")}
        >
          <FolderSearch className="h-4 w-4" />
        </Button>
        <Button
          type="button"
          variant="outline"
          size="icon"
          onClick={onReset}
          title={t("settings.resetDefault")}
        >
          <Undo2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
