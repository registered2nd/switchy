import { useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import { motion, AnimatePresence } from "framer-motion";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import {
  Plus,
  ArrowLeft,
  Download,
  FolderOpen,
  KeyRound,
  Shield,
  Cpu,
} from "lucide-react";
import type { EnvConflict } from "@/types/env";
import { useSettingsQuery } from "@/lib/query";
import { providersApi, settingsApi } from "@/lib/api";
import { checkAllEnvConflicts } from "@/lib/api/env";
import { useProviderActions } from "@/hooks/useProviderActions";
import { useOpenClawHealth } from "@/hooks/useOpenClaw";
import { isTextEditableTarget } from "@/utils/domUtils";
import { isWindows, isLinux } from "@/lib/platform";
import { useTheme } from "@/components/theme-provider";
import { AppRail, type SettingsSection } from "@/components/layout/AppRail";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { EnvWarningBanner } from "@/components/env/EnvWarningBanner";
import { UniversalProviderPanel } from "@/components/universal";
import { Button } from "@/components/ui/button";
import WorkspaceFilesPanel from "@/components/workspace/WorkspaceFilesPanel";
import EnvPanel from "@/components/openclaw/EnvPanel";
import ToolsPanel from "@/components/openclaw/ToolsPanel";
import AgentsDefaultsPanel from "@/components/openclaw/AgentsDefaultsPanel";
import OpenClawHealthBanner from "@/components/openclaw/OpenClawHealthBanner";
import { ProgramAccounts } from "@/components/companies/ProgramAccounts";
import {
  AddAccountMenu,
  CompanyView,
} from "@/components/companies/CompanyView";
import { useSidebarEntries } from "@/components/companies/useSidebarEntries";
import {
  companyById,
  COMPANIES,
  OTHER_COMPANY,
  type SidebarEntryId,
} from "@/config/companies";

type View =
  | "providers"
  | "settings"
  | "universal"
  | "openclaw"
  | "workspace"
  | "openclawEnv"
  | "openclawTools"
  | "openclawAgents";

const DRAG_BAR_HEIGHT = isWindows() || isLinux() ? 0 : 28; // px

const ENTRY_STORAGE_KEY = "switchy-last-company";
const VIEW_STORAGE_KEY = "switchy-last-view";
const VALID_VIEWS: View[] = [
  "providers",
  "settings",
  "universal",
  "openclaw",
  "workspace",
  "openclawEnv",
  "openclawTools",
  "openclawAgents",
];

function readStorage(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Remembering the last page is only a convenience.
  }
}

const getInitialEntry = (): SidebarEntryId => {
  const saved = readStorage(ENTRY_STORAGE_KEY);
  if (saved && (saved === OTHER_COMPANY || companyById(saved))) {
    return saved as SidebarEntryId;
  }
  return "anthropic";
};

const getInitialView = (): View => {
  const saved = readStorage(VIEW_STORAGE_KEY) as View | null;
  return saved && VALID_VIEWS.includes(saved) ? saved : "providers";
};

/** OpenClaw's own page: its providers and its gateway settings. */
function OpenClawHeaderActions({ onOpen }: { onOpen: (view: View) => void }) {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const { addProvider } = useProviderActions("openclaw");
  const queryClient = useQueryClient();
  const importFromLive = async () => {
    try {
      const count = await providersApi.importOpenClawFromLive();
      await queryClient.invalidateQueries({
        queryKey: ["providers", "openclaw"],
      });
      if (count > 0) toast.success(t("provider.importCurrentDescription"));
      else toast.info(t("provider.noProviders"));
    } catch (error) {
      toast.error(String(error));
    }
  };
  return (
    <div
      className="ml-auto flex items-center gap-1"
      style={{ WebkitAppRegion: "no-drag" } as any}
    >
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onOpen("workspace")}
        title={t("workspace.manage")}
      >
        <FolderOpen className="h-4 w-4" />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onOpen("openclawEnv")}
        title={t("openclaw.env.title")}
      >
        <KeyRound className="h-4 w-4" />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onOpen("openclawTools")}
        title={t("openclaw.tools.title")}
      >
        <Shield className="h-4 w-4" />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onOpen("openclawAgents")}
        title={t("openclaw.agents.title")}
      >
        <Cpu className="h-4 w-4" />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void importFromLive()}
        title={t("account.importOpenClaw", {
          defaultValue: "Import the providers in OpenClaw's config",
        })}
      >
        <Download className="h-4 w-4" />
      </Button>
      <Button size="sm" onClick={() => setAdding(true)} className="gap-1.5">
        <Plus className="h-4 w-4" />
        {t("provider.addProvider", { defaultValue: "Add" })}
      </Button>
      <AddProviderDialog
        open={adding}
        onOpenChange={setAdding}
        appId="openclaw"
        onSubmit={addProvider}
      />
    </div>
  );
}

function App() {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const { setTheme } = useTheme();

  const [activeEntry, setActiveEntry] =
    useState<SidebarEntryId>(getInitialEntry);
  const [currentView, setCurrentView] = useState<View>(getInitialView);
  const [settingsDefaultTab, setSettingsDefaultTab] =
    useState<SettingsSection>("general");
  const entries = useSidebarEntries(
    t("account.other", { defaultValue: "Other" }),
  );

  const notifiedCodexUpdates = useRef("");
  useEffect(() => {
    let live = true;
    const check = () => {
      settingsApi
        .getCodexTerminalUpdates()
        .then((updates) => {
          if (!live) return;
          const pending = updates.filter((update) => !update.queued);
          if (pending.length === 0) return;
          const signature = pending
            .map(
              (update) =>
                `${update.socket}:${update.installedVersion}:${update.catalogChanged}`,
            )
            .sort()
            .join("|");
          if (signature === notifiedCodexUpdates.current) return;
          notifiedCodexUpdates.current = signature;
          toast.warning(t("proxy.accountPool.codexUpdatesNotice"), {
            duration: 12000,
            closeButton: true,
            action: {
              label: t("proxy.accountPool.codexUpdatesOpenPool"),
              onClick: () => {
                setSettingsDefaultTab("pool");
                setCurrentView("settings");
              },
            },
          });
        })
        .catch((error) =>
          console.error("Failed to check Codex terminals:", error),
        );
    };
    check();
    const timer = window.setInterval(check, 60_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [t]);

  useEffect(() => {
    writeStorage(VIEW_STORAGE_KEY, currentView);
  }, [currentView]);

  const { data: settingsData } = useSettingsQuery();
  const openclawVisible = Boolean(settingsData?.visibleApps?.openclaw);

  // A hidden or emptied entry falls back to the first one listed.
  useEffect(() => {
    if (
      entries.length > 0 &&
      !entries.some((entry) => entry.id === activeEntry)
    ) {
      setActiveEntry(entries[0].id);
    }
  }, [entries, activeEntry]);

  useEffect(() => {
    if (settingsData && !openclawVisible && currentView === "openclaw") {
      setCurrentView("providers");
    }
  }, [settingsData, openclawVisible, currentView]);

  const isOpenClawView =
    currentView === "openclaw" ||
    currentView === "workspace" ||
    currentView === "openclawEnv" ||
    currentView === "openclawTools" ||
    currentView === "openclawAgents";
  const { data: openclawHealthWarnings = [] } =
    useOpenClawHealth(isOpenClawView);

  const [envConflicts, setEnvConflicts] = useState<EnvConflict[]>([]);
  const [showEnvBanner, setShowEnvBanner] = useState(false);

  // A switch from the tray, the proxy or an agent: reload that program.
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    providersApi
      .onSwitched(async (event) => {
        await queryClient.invalidateQueries({
          queryKey: ["providers", event.appType],
        });
      })
      .then((fn) => {
        unsubscribe = fn;
      })
      .catch((error) =>
        console.error("[App] Failed to subscribe provider switch event", error),
      );
    return () => {
      unsubscribe?.();
    };
  }, [queryClient]);

  // Theme and language set by a command (set_window_appearance).
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    const setup = async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unsubscribe = await listen<{
        theme?: string | null;
        language?: string | null;
      }>("window-appearance", (event) => {
        const { theme, language } = event.payload ?? {};
        if (theme === "light" || theme === "dark" || theme === "system") {
          setTheme(theme);
        }
        if (language) {
          void i18n.changeLanguage(language);
          writeStorage("language", language);
          void queryClient.invalidateQueries({ queryKey: ["settings"] });
        }
      });
    };
    setup().catch((error) =>
      console.error("[App] Failed to subscribe to window-appearance", error),
    );
    return () => {
      unsubscribe?.();
    };
  }, [i18n, queryClient, setTheme]);

  // An agent changed something through the control routes: reload it all.
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    providersApi
      .onAgentInvoked(() => {
        queryClient.invalidateQueries();
      })
      .then((fn) => {
        unsubscribe = fn;
      })
      .catch((error) =>
        console.error("[App] Failed to subscribe to agent commands", error),
      );
    return () => {
      unsubscribe?.();
    };
  }, [queryClient]);

  // A sign-in filed with a card: show it signed in and clear the failures
  // the refused login left, without waiting for the next poll.
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    providersApi
      .onAccountSignedIn((event) => {
        queryClient.invalidateQueries({ queryKey: ["subscription", "quota"] });
        queryClient.invalidateQueries({
          queryKey: ["providerHealth", event.providerId],
        });
      })
      .then((fn) => {
        unsubscribe = fn;
      })
      .catch((error) =>
        console.error("[App] Failed to subscribe to sign-in events", error),
      );
    return () => {
      unsubscribe?.();
    };
  }, [queryClient]);

  // Name the account when a request finds its login refused for good. The
  // proxy moves on to another account; this says which one needs signing in.
  const signedOutNoticeAt = useRef<Record<string, number>>({});
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    providersApi
      .onAccountNeedsSignIn((event) => {
        const now = Date.now();
        const last = signedOutNoticeAt.current[event.providerId] ?? 0;
        if (now - last < 30 * 60 * 1000) return;
        signedOutNoticeAt.current[event.providerId] = now;
        toast.warning(
          t("notifications.accountSignedOut", {
            account: event.account || event.providerName,
            app: t(`apps.${event.appType}`, { defaultValue: event.appType }),
          }),
          {
            description: t(`subscription.signedOutHint.${event.appType}`, {
              defaultValue: t("subscription.signedOutHint.claude"),
            }),
            duration: 12000,
            closeButton: true,
          },
        );
        queryClient.invalidateQueries({ queryKey: ["subscription", "quota"] });
      })
      .then((fn) => {
        unsubscribe = fn;
      })
      .catch((error) =>
        console.error("[App] Failed to subscribe to sign-in notices", error),
      );
    return () => {
      unsubscribe?.();
    };
  }, [queryClient, t]);

  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    const setupListener = async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        unsubscribe = await listen("universal-provider-synced", async () => {
          await queryClient.invalidateQueries({ queryKey: ["providers"] });
          try {
            await providersApi.updateTrayMenu();
          } catch (error) {
            console.error("[App] Failed to update tray menu", error);
          }
        });
      } catch (error) {
        console.error(
          "[App] Failed to subscribe universal-provider-synced event",
          error,
        );
      }
    };
    setupListener();
    return () => {
      unsubscribe?.();
    };
  }, [queryClient]);

  useEffect(() => {
    const checkEnvOnStartup = async () => {
      try {
        const allConflicts = await checkAllEnvConflicts();
        const flatConflicts = Object.values(allConflicts).flat();
        if (flatConflicts.length > 0) {
          setEnvConflicts(flatConflicts);
          const dismissed = sessionStorage.getItem("env_banner_dismissed");
          if (!dismissed) {
            setShowEnvBanner(true);
          }
        }
      } catch (error) {
        console.error(
          "[App] Failed to check environment conflicts on startup:",
          error,
        );
      }
    };
    checkEnvOnStartup();
  }, []);

  const currentViewRef = useRef(currentView);
  useEffect(() => {
    currentViewRef.current = currentView;
  }, [currentView]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "," && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setCurrentView("settings");
        return;
      }
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.body.style.overflow === "hidden") return;
      const view = currentViewRef.current;
      if (view === "providers") return;
      if (isTextEditableTarget(event.target)) return;
      event.preventDefault();
      setCurrentView("providers");
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  const handleImportSuccess = async () => {
    try {
      await queryClient.invalidateQueries({
        queryKey: ["providers"],
        refetchType: "all",
      });
      await queryClient.invalidateQueries({ queryKey: ["accounts"] });
    } catch (error) {
      console.error("[App] Failed to refresh providers after import", error);
    }
    try {
      await providersApi.updateTrayMenu();
    } catch (error) {
      console.error("[App] Failed to refresh tray menu", error);
    }
  };

  const renderContent = () => {
    const content = (() => {
      switch (currentView) {
        case "settings":
          return (
            <SettingsPage
              open={true}
              onOpenChange={() => setCurrentView("providers")}
              onImportSuccess={handleImportSuccess}
              activeTab={settingsDefaultTab}
              onOpenOpenClaw={() => setCurrentView("openclaw")}
            />
          );
        case "universal":
          return (
            <div className="px-6 pt-4">
              <UniversalProviderPanel />
            </div>
          );
        case "workspace":
          return <WorkspaceFilesPanel />;
        case "openclawEnv":
          return <EnvPanel />;
        case "openclawTools":
          return <ToolsPanel />;
        case "openclawAgents":
          return <AgentsDefaultsPanel />;
        case "openclaw":
          return (
            <div className="px-6 pb-12">
              <ProgramAccounts
                appId="openclaw"
                showProgram={false}
                filter={() => true}
              />
            </div>
          );
        default:
          return (
            <div className="px-6 flex flex-col flex-1 min-h-0 overflow-hidden">
              <div className="flex-1 overflow-y-auto overflow-x-hidden pb-12 px-1">
                <AnimatePresence mode="wait">
                  <motion.div
                    key={activeEntry}
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.15 }}
                    className="mt-1"
                  >
                    <CompanyView entry={activeEntry} />
                  </motion.div>
                </AnimatePresence>
              </div>
            </div>
          );
      }
    })();

    return (
      <AnimatePresence mode="wait">
        <motion.div
          key={currentView}
          className="flex-1 min-h-0"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
        >
          {content}
        </motion.div>
      </AnimatePresence>
    );
  };

  const otherViewTitle =
    currentView === "universal"
      ? t("universalProvider.title", { defaultValue: "Universal Provider" })
      : currentView === "openclaw"
        ? "OpenClaw"
        : currentView === "workspace"
          ? t("workspace.title")
          : currentView === "openclawEnv"
            ? t("openclaw.env.title")
            : currentView === "openclawTools"
              ? t("openclaw.tools.title")
              : currentView === "openclawAgents"
                ? t("openclaw.agents.title")
                : "";

  const entryName =
    activeEntry === OTHER_COMPANY
      ? t("account.other", { defaultValue: "Other" })
      : (COMPANIES.find((company) => company.id === activeEntry)?.name ?? "");

  const pageHeader =
    currentView === "settings" ? null : currentView !== "providers" ? (
      <>
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() =>
              setCurrentView(
                currentView === "openclaw" || !isOpenClawView
                  ? "providers"
                  : "openclaw",
              )
            }
            style={{ WebkitAppRegion: "no-drag" } as any}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <h1 className="font-display text-[22px] font-semibold tracking-tight">
            {otherViewTitle}
          </h1>
        </div>
        {currentView === "openclaw" && (
          <OpenClawHeaderActions onOpen={setCurrentView} />
        )}
      </>
    ) : (
      <>
        <h1 className="font-display text-[22px] font-semibold tracking-tight">
          {entryName}
        </h1>
        <div
          className="ml-auto flex items-center gap-2"
          style={{ WebkitAppRegion: "no-drag" } as any}
        >
          <AddAccountMenu
            entry={activeEntry}
            onOpenUniversal={() => setCurrentView("universal")}
          />
        </div>
      </>
    );

  return (
    <div
      className="flex h-screen overflow-hidden bg-background text-foreground selection:bg-primary/30"
      style={{ overflowX: "hidden", paddingTop: DRAG_BAR_HEIGHT }}
    >
      <div
        className="fixed top-0 left-0 right-0 z-[60]"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag", height: DRAG_BAR_HEIGHT } as any}
      />

      <AppRail
        view={
          currentView === "providers"
            ? "providers"
            : currentView === "settings"
              ? "settings"
              : "other"
        }
        entries={entries}
        activeEntry={activeEntry}
        onSelectEntry={(entry) => {
          setActiveEntry(entry);
          setCurrentView("providers");
        }}
        settingsSection={settingsDefaultTab}
        onOpenSettings={(section) => {
          setSettingsDefaultTab(section);
          setCurrentView("settings");
        }}
        onBack={() => setCurrentView("providers")}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {showEnvBanner && envConflicts.length > 0 && (
          <EnvWarningBanner
            conflicts={envConflicts}
            onDismiss={() => {
              setShowEnvBanner(false);
              sessionStorage.setItem("env_banner_dismissed", "true");
            }}
            onDeleted={async () => {
              try {
                const allConflicts = await checkAllEnvConflicts();
                const flatConflicts = Object.values(allConflicts).flat();
                setEnvConflicts(flatConflicts);
                if (flatConflicts.length === 0) {
                  setShowEnvBanner(false);
                }
              } catch (error) {
                console.error(
                  "[App] Failed to re-check conflicts after deletion:",
                  error,
                );
              }
            }}
          />
        )}

        {pageHeader && (
          <header
            className="flex h-14 shrink-0 items-center gap-3 px-6"
            data-tauri-drag-region
            style={{ WebkitAppRegion: "drag" } as any}
          >
            {pageHeader}
          </header>
        )}

        <main className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {isOpenClawView && openclawHealthWarnings.length > 0 && (
            <OpenClawHealthBanner warnings={openclawHealthWarnings} />
          )}
          {renderContent()}
        </main>
      </div>
    </div>
  );
}

export default App;
