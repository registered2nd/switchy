import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, BarChart2, Settings as SettingsIcon } from "lucide-react";
import type { AppId } from "@/lib/api";
import type { VisibleApps } from "@/types";
import { ProviderIcon } from "@/components/ProviderIcon";
import { cn } from "@/lib/utils";

// OpenClaw is hidden by default (Settings → App visibility can bring it back).
const ALL_APPS: AppId[] = [
  "claude",
  "codex",
  "gemini",
  "kimi",
  "opencode",
  "openclaw",
];
const STORAGE_KEY = "switchy-last-app";

export const APP_ICON: Record<AppId, string> = {
  claude: "claude",
  codex: "openai",
  gemini: "gemini",
  kimi: "kimi",
  opencode: "opencode",
  openclaw: "openclaw",
};

export const APP_NAME: Record<AppId, string> = {
  claude: "Claude",
  codex: "Codex",
  gemini: "Gemini",
  kimi: "Kimi",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
};

export const SETTINGS_SECTIONS = [
  "general",
  "pool",
  "auth",
  "data",
  "advanced",
  "usage",
  "about",
] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

interface AppRailProps {
  view: "providers" | "settings" | "other";
  activeApp: AppId;
  visibleApps?: VisibleApps;
  onSelectApp: (app: AppId) => void;
  settingsSection: SettingsSection;
  onOpenSettings: (section: SettingsSection) => void;
  onBack: () => void;
}

function RailItem({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-left text-[13.5px] transition-colors",
        active
          ? "bg-card font-medium text-foreground"
          : "text-muted-foreground hover:bg-card/60 hover:text-foreground",
      )}
    >
      {active && (
        <span className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-full bg-primary" />
      )}
      {icon}
      <span className="truncate">{children}</span>
    </button>
  );
}

export function AppRail({
  view,
  activeApp,
  visibleApps,
  onSelectApp,
  settingsSection,
  onOpenSettings,
  onBack,
}: AppRailProps) {
  const { t } = useTranslation();
  const apps = ALL_APPS.filter((app) => !visibleApps || visibleApps[app]);

  const sectionLabel: Record<SettingsSection, string> = {
    general: t("settings.tabGeneral"),
    pool: t("settings.tabPool"),
    auth: t("settings.tabAuth", { defaultValue: "Sign-in" }),
    data: t("settings.advanced.data.title"),
    advanced: t("settings.tabAdvanced"),
    usage: t("usage.title"),
    about: t("common.about"),
  };

  return (
    <nav
      className="flex h-full w-[200px] shrink-0 flex-col border-r border-border bg-rail px-3 pb-3"
      aria-label="Switchy"
    >
      <div
        className="flex h-14 shrink-0 items-center px-3"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as any}
      >
        <span className="font-display text-[22px] font-semibold tracking-tight text-foreground">
          Switchy
        </span>
      </div>

      {view === "settings" ? (
        <div className="flex flex-1 flex-col gap-0.5">
          <RailItem
            active={false}
            onClick={onBack}
            icon={<ArrowLeft className="h-4 w-4" />}
          >
            {t("rail.accounts", { defaultValue: "Accounts" })}
          </RailItem>
          <div className="my-2 h-px bg-border" />
          {SETTINGS_SECTIONS.map((section) => (
            <RailItem
              key={section}
              active={settingsSection === section}
              onClick={() => onOpenSettings(section)}
            >
              {sectionLabel[section]}
            </RailItem>
          ))}
        </div>
      ) : (
        <>
          <div className="flex flex-1 flex-col gap-0.5">
            {apps.map((app) => (
              <RailItem
                key={app}
                active={view === "providers" && activeApp === app}
                onClick={() => {
                  localStorage.setItem(STORAGE_KEY, app);
                  onSelectApp(app);
                }}
                icon={
                  <ProviderIcon icon={APP_ICON[app]} name={APP_NAME[app]} size={17} />
                }
              >
                {APP_NAME[app]}
              </RailItem>
            ))}
          </div>
          <div className="flex flex-col gap-0.5 border-t border-border pt-3">
            <RailItem
              active={false}
              onClick={() => onOpenSettings("usage")}
              icon={<BarChart2 className="h-4 w-4" />}
            >
              {sectionLabel.usage}
            </RailItem>
            <RailItem
              active={false}
              onClick={() => onOpenSettings("general")}
              icon={<SettingsIcon className="h-4 w-4" />}
            >
              {t("common.settings")}
            </RailItem>
          </div>
        </>
      )}
    </nav>
  );
}
