import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { homeDir, join } from "@tauri-apps/api/path";
import { settingsApi, type AppId } from "@/lib/api";
import type { SettingsFormState } from "./useSettingsForm";

type DirectoryKey =
  | "appConfig"
  | "claude"
  | "codex"
  | "gemini"
  | "kimi"
  | "opencode";

export interface ResolvedDirectories {
  appConfig: string;
  claude: string;
  codex: string;
  gemini: string;
  kimi: string;
  opencode: string;
}

const sanitizeDir = (value?: string | null): string | undefined => {
  if (!value) return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
};

const computeDefaultAppConfigDir = async (): Promise<string | undefined> => {
  try {
    const home = await homeDir();
    return await join(home, ".switchy");
  } catch (error) {
    console.error(
      "[useDirectorySettings] Failed to resolve default app config dir",
      error,
    );
    return undefined;
  }
};

const computeDefaultConfigDir = async (
  app: AppId,
): Promise<string | undefined> => {
  try {
    const home = await homeDir();
    const folder =
      app === "claude"
        ? ".claude"
        : app === "codex"
          ? ".codex"
          : app === "gemini"
            ? ".gemini"
            : app === "kimi"
              ? ".kimi-code"
              : ".config/opencode";
    return await join(home, folder);
  } catch (error) {
    console.error(
      "[useDirectorySettings] Failed to resolve default config dir",
      error,
    );
    return undefined;
  }
};

export interface UseDirectorySettingsProps {
  settings: SettingsFormState | null;
  onUpdateSettings: (updates: Partial<SettingsFormState>) => void;
}

export interface UseDirectorySettingsResult {
  appConfigDir?: string;
  resolvedDirs: ResolvedDirectories;
  isLoading: boolean;
  initialAppConfigDir?: string;
  claudeMirrorDir?: string;
  codexMirrorDir?: string;
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
  resetAllDirectories: (
    claudeDir?: string,
    claudeMirrorDir?: string,
    codexDir?: string,
    geminiDir?: string,
    opencodeDir?: string,
    codexMirrorDir?: string,
    kimiDir?: string,
  ) => void;
}

/**
 * useDirectorySettings - 目录管理
 * 负责：
 * - appConfigDir 状态
 * - resolvedDirs 状态
 * - 目录选择（browse）
 * - 目录重置
 * - 默认值计算
 */
export function useDirectorySettings({
  settings,
  onUpdateSettings,
}: UseDirectorySettingsProps): UseDirectorySettingsResult {
  const { t } = useTranslation();

  const [appConfigDir, setAppConfigDir] = useState<string | undefined>(
    undefined,
  );
  const [claudeMirrorDir, setClaudeMirrorDir] = useState<string | undefined>(
    undefined,
  );
  const [defaultClaudeMirrorDir, setDefaultClaudeMirrorDir] = useState<
    string | undefined
  >(undefined);
  const [codexMirrorDir, setCodexMirrorDir] = useState<string | undefined>(
    undefined,
  );
  const [defaultCodexMirrorDir, setDefaultCodexMirrorDir] = useState<
    string | undefined
  >(undefined);
  const [resolvedDirs, setResolvedDirs] = useState<ResolvedDirectories>({
    appConfig: "",
    claude: "",
    codex: "",
    gemini: "",
    kimi: "",
    opencode: "",
  });
  const [isLoading, setIsLoading] = useState(true);

  const defaultsRef = useRef<ResolvedDirectories>({
    appConfig: "",
    claude: "",
    codex: "",
    gemini: "",
    kimi: "",
    opencode: "",
  });
  const initialAppConfigDirRef = useRef<string | undefined>(undefined);
  const mirrorSeededRef = useRef(false);
  const codexMirrorSeededRef = useRef(false);

  // 加载目录信息
  useEffect(() => {
    let active = true;
    setIsLoading(true);

    const load = async () => {
      try {
        const [
          overrideRaw,
          claudeDir,
          codexDir,
          geminiDir,
          opencodeDir,
          defaultClaudeMirrorDir,
          defaultAppConfig,
          defaultClaudeDir,
          defaultCodexDir,
          defaultGeminiDir,
          defaultOpencodeDir,
          defaultCodexMirrorDir,
          kimiDir,
          defaultKimiDir,
        ] = await Promise.all([
          settingsApi.getAppConfigDirOverride(),
          settingsApi.getConfigDir("claude"),
          settingsApi.getConfigDir("codex"),
          settingsApi.getConfigDir("gemini"),
          settingsApi.getConfigDir("opencode"),
          settingsApi.getDefaultClaudeMirrorDir(),
          computeDefaultAppConfigDir(),
          computeDefaultConfigDir("claude"),
          computeDefaultConfigDir("codex"),
          computeDefaultConfigDir("gemini"),
          computeDefaultConfigDir("opencode"),
          settingsApi.getDefaultCodexMirrorDir(),
          settingsApi.getConfigDir("kimi"),
          computeDefaultConfigDir("kimi"),
        ]);

        if (!active) return;

        const normalizedOverride = sanitizeDir(overrideRaw ?? undefined);
        const normalizedClaudeMirror = sanitizeDir(
          settings?.claudeMirrorConfigDir ??
            defaultClaudeMirrorDir ??
            undefined,
        );
        const normalizedCodexMirror = sanitizeDir(
          settings?.codexMirrorConfigDir ?? defaultCodexMirrorDir ?? undefined,
        );

        defaultsRef.current = {
          appConfig: defaultAppConfig ?? "",
          claude: defaultClaudeDir ?? "",
          codex: defaultCodexDir ?? "",
          gemini: defaultGeminiDir ?? "",
          kimi: defaultKimiDir ?? "",
          opencode: defaultOpencodeDir ?? "",
        };

        setAppConfigDir(normalizedOverride);
        setDefaultClaudeMirrorDir(
          sanitizeDir(defaultClaudeMirrorDir ?? undefined),
        );
        setClaudeMirrorDir(normalizedClaudeMirror);
        setDefaultCodexMirrorDir(
          sanitizeDir(defaultCodexMirrorDir ?? undefined),
        );
        setCodexMirrorDir(normalizedCodexMirror);
        initialAppConfigDirRef.current = normalizedOverride;

        setResolvedDirs({
          appConfig: normalizedOverride ?? defaultsRef.current.appConfig,
          claude: claudeDir || defaultsRef.current.claude,
          codex: codexDir || defaultsRef.current.codex,
          gemini: geminiDir || defaultsRef.current.gemini,
          kimi: kimiDir || defaultsRef.current.kimi,
          opencode: opencodeDir || defaultsRef.current.opencode,
        });
      } catch (error) {
        console.error(
          "[useDirectorySettings] Failed to load directory info",
          error,
        );
      } finally {
        if (active) {
          setIsLoading(false);
        }
      }
    };

    void load();
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!settings) return;
    const explicitMirror = sanitizeDir(settings.claudeMirrorConfigDir);
    if (explicitMirror) {
      setClaudeMirrorDir(explicitMirror);
      mirrorSeededRef.current = true;
      return;
    }

    if (!mirrorSeededRef.current && defaultClaudeMirrorDir) {
      setClaudeMirrorDir(defaultClaudeMirrorDir);
      onUpdateSettings({ claudeMirrorConfigDir: defaultClaudeMirrorDir });
      mirrorSeededRef.current = true;
    }
  }, [claudeMirrorDir, defaultClaudeMirrorDir, onUpdateSettings, settings]);

  useEffect(() => {
    if (!settings) return;
    const explicitMirror = sanitizeDir(settings.codexMirrorConfigDir);
    if (explicitMirror) {
      setCodexMirrorDir(explicitMirror);
      codexMirrorSeededRef.current = true;
      return;
    }

    if (!codexMirrorSeededRef.current && defaultCodexMirrorDir) {
      setCodexMirrorDir(defaultCodexMirrorDir);
      onUpdateSettings({ codexMirrorConfigDir: defaultCodexMirrorDir });
      codexMirrorSeededRef.current = true;
    }
  }, [codexMirrorDir, defaultCodexMirrorDir, onUpdateSettings, settings]);

  const updateDirectoryState = useCallback(
    (key: DirectoryKey, value?: string) => {
      const sanitized = sanitizeDir(value);
      if (key === "appConfig") {
        setAppConfigDir(sanitized);
      } else {
        onUpdateSettings(
          key === "claude"
            ? { claudeConfigDir: sanitized }
            : key === "codex"
              ? { codexConfigDir: sanitized }
              : key === "gemini"
                ? { geminiConfigDir: sanitized }
                : key === "kimi"
                  ? { kimiConfigDir: sanitized }
                  : { opencodeConfigDir: sanitized },
        );
      }

      setResolvedDirs((prev) => ({
        ...prev,
        [key]: sanitized ?? defaultsRef.current[key],
      }));
    },
    [onUpdateSettings],
  );

  const updateAppConfigDir = useCallback(
    (value?: string) => {
      updateDirectoryState("appConfig", value);
    },
    [updateDirectoryState],
  );

  const updateClaudeMirrorDir = useCallback(
    (value?: string) => {
      const sanitized = sanitizeDir(value);
      setClaudeMirrorDir(sanitized);
      onUpdateSettings({ claudeMirrorConfigDir: sanitized });
    },
    [onUpdateSettings],
  );

  const updateCodexMirrorDir = useCallback(
    (value?: string) => {
      const sanitized = sanitizeDir(value);
      setCodexMirrorDir(sanitized);
      onUpdateSettings({ codexMirrorConfigDir: sanitized });
    },
    [onUpdateSettings],
  );

  const updateDirectory = useCallback(
    (app: AppId, value?: string) => {
      updateDirectoryState(
        app === "claude"
          ? "claude"
          : app === "codex"
            ? "codex"
            : app === "gemini"
              ? "gemini"
              : app === "kimi"
                ? "kimi"
                : "opencode",
        value,
      );
    },
    [updateDirectoryState],
  );

  const browseDirectory = useCallback(
    async (app: AppId) => {
      const key: DirectoryKey =
        app === "claude"
          ? "claude"
          : app === "codex"
            ? "codex"
            : app === "gemini"
              ? "gemini"
              : app === "kimi"
                ? "kimi"
                : "opencode";
      const currentValue =
        key === "claude"
          ? (settings?.claudeConfigDir ?? resolvedDirs.claude)
          : key === "codex"
            ? (settings?.codexConfigDir ?? resolvedDirs.codex)
            : key === "gemini"
              ? (settings?.geminiConfigDir ?? resolvedDirs.gemini)
              : key === "kimi"
                ? (settings?.kimiConfigDir ?? resolvedDirs.kimi)
                : (settings?.opencodeConfigDir ?? resolvedDirs.opencode);

      try {
        const picked = await settingsApi.selectConfigDirectory(currentValue);
        const sanitized = sanitizeDir(picked ?? undefined);
        if (!sanitized) return;
        updateDirectoryState(key, sanitized);
      } catch (error) {
        console.error("[useDirectorySettings] Failed to pick directory", error);
        toast.error(
          t("settings.selectFileFailed", {
            defaultValue: "选择目录失败",
          }),
        );
      }
    },
    [settings, resolvedDirs, t, updateDirectoryState],
  );

  const browseAppConfigDir = useCallback(async () => {
    const currentValue = appConfigDir ?? resolvedDirs.appConfig;
    try {
      const picked = await settingsApi.selectConfigDirectory(currentValue);
      const sanitized = sanitizeDir(picked ?? undefined);
      if (!sanitized) return;
      updateDirectoryState("appConfig", sanitized);
    } catch (error) {
      console.error(
        "[useDirectorySettings] Failed to pick app config directory",
        error,
      );
      toast.error(
        t("settings.selectFileFailed", {
          defaultValue: "选择目录失败",
        }),
      );
    }
  }, [appConfigDir, resolvedDirs.appConfig, t, updateDirectoryState]);

  const browseClaudeMirrorDir = useCallback(async () => {
    const currentValue =
      settings?.claudeMirrorConfigDir ?? claudeMirrorDir ?? "";
    try {
      const picked = await settingsApi.selectConfigDirectory(currentValue);
      const sanitized = sanitizeDir(picked ?? undefined);
      if (!sanitized) return;
      updateClaudeMirrorDir(sanitized);
    } catch (error) {
      console.error(
        "[useDirectorySettings] Failed to pick Claude mirror directory",
        error,
      );
      toast.error(
        t("settings.selectFileFailed", {
          defaultValue: "选择目录失败",
        }),
      );
    }
  }, [
    claudeMirrorDir,
    settings?.claudeMirrorConfigDir,
    t,
    updateClaudeMirrorDir,
  ]);

  const browseCodexMirrorDir = useCallback(async () => {
    const currentValue = settings?.codexMirrorConfigDir ?? codexMirrorDir ?? "";
    try {
      const picked = await settingsApi.selectConfigDirectory(currentValue);
      const sanitized = sanitizeDir(picked ?? undefined);
      if (!sanitized) return;
      updateCodexMirrorDir(sanitized);
    } catch (error) {
      console.error(
        "[useDirectorySettings] Failed to pick Codex mirror directory",
        error,
      );
      toast.error(
        t("settings.selectFileFailed", {
          defaultValue: "选择目录失败",
        }),
      );
    }
  }, [codexMirrorDir, settings?.codexMirrorConfigDir, t, updateCodexMirrorDir]);

  const resetDirectory = useCallback(
    async (app: AppId) => {
      const key: DirectoryKey =
        app === "claude"
          ? "claude"
          : app === "codex"
            ? "codex"
            : app === "gemini"
              ? "gemini"
              : app === "kimi"
                ? "kimi"
                : "opencode";
      if (!defaultsRef.current[key]) {
        const fallback = await computeDefaultConfigDir(app);
        if (fallback) {
          defaultsRef.current = {
            ...defaultsRef.current,
            [key]: fallback,
          };
        }
      }
      updateDirectoryState(key, undefined);
    },
    [updateDirectoryState],
  );

  const resetAppConfigDir = useCallback(async () => {
    if (!defaultsRef.current.appConfig) {
      const fallback = await computeDefaultAppConfigDir();
      if (fallback) {
        defaultsRef.current = {
          ...defaultsRef.current,
          appConfig: fallback,
        };
      }
    }
    updateDirectoryState("appConfig", undefined);
  }, [updateDirectoryState]);

  const resetClaudeMirrorDir = useCallback(async () => {
    updateClaudeMirrorDir(undefined);
  }, [updateClaudeMirrorDir]);

  const resetCodexMirrorDir = useCallback(async () => {
    updateCodexMirrorDir(undefined);
  }, [updateCodexMirrorDir]);

  const resetAllDirectories = useCallback(
    (
      claudeDir?: string,
      claudeMirrorDirValue?: string,
      codexDir?: string,
      geminiDir?: string,
      opencodeDir?: string,
      codexMirrorDirValue?: string,
      kimiDir?: string,
    ) => {
      setAppConfigDir(initialAppConfigDirRef.current);
      setClaudeMirrorDir(claudeMirrorDirValue);
      setCodexMirrorDir(codexMirrorDirValue);
      setResolvedDirs({
        appConfig:
          initialAppConfigDirRef.current ?? defaultsRef.current.appConfig,
        claude: claudeDir ?? defaultsRef.current.claude,
        codex: codexDir ?? defaultsRef.current.codex,
        gemini: geminiDir ?? defaultsRef.current.gemini,
        kimi: kimiDir ?? defaultsRef.current.kimi,
        opencode: opencodeDir ?? defaultsRef.current.opencode,
      });
    },
    [],
  );

  return {
    appConfigDir,
    resolvedDirs,
    isLoading,
    initialAppConfigDir: initialAppConfigDirRef.current,
    claudeMirrorDir,
    codexMirrorDir,
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
  };
}
