import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeft,
  BarChart2,
  MoreHorizontal,
  Settings as SettingsIcon,
} from "lucide-react";
import type { AppId } from "@/lib/api";
import type { SidebarEntryId } from "@/config/companies";
import type { SidebarEntry } from "@/components/companies/useSidebarEntries";
import { ProviderIcon } from "@/components/ProviderIcon";
import { cn } from "@/lib/utils";

const STORAGE_KEY = "switchy-last-company";

/** The programs' display names, kept for the places that still name a program. */
export const APP_NAME: Record<AppId, string> = {
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  kimi: "Kimi Code",
  cursor: "Cursor CLI",
  antigravity: "Antigravity",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
};

export const SETTINGS_SECTIONS = [
  "general",
  "programs",
  "pool",
  "data",
  "advanced",
  "usage",
  "about",
] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

interface AppRailProps {
  view: "providers" | "settings" | "other";
  entries: SidebarEntry[];
  activeEntry: SidebarEntryId;
  onSelectEntry: (entry: SidebarEntryId) => void;
  settingsSection: SettingsSection;
  onOpenSettings: (section: SettingsSection) => void;
  onBack: () => void;
}

function RailItem({
  active,
  onClick,
  icon,
  inUse = false,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon?: ReactNode;
  inUse?: boolean;
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
      {inUse && (
        <span
          aria-hidden
          className="ml-auto h-1.5 w-1.5 shrink-0 rounded-full bg-primary/70"
        />
      )}
    </button>
  );
}

export function AppRail({
  view,
  entries,
  activeEntry,
  onSelectEntry,
  settingsSection,
  onOpenSettings,
  onBack,
}: AppRailProps) {
  const { t } = useTranslation();

  const sectionLabel: Record<SettingsSection, string> = {
    general: t("settings.tabGeneral"),
    programs: t("settings.tabPrograms", { defaultValue: "Programs" }),
    pool: t("settings.tabPool"),
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
          <div className="-mr-2 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto pr-2">
            {entries.map((entry) => (
              <RailItem
                key={entry.id}
                active={view === "providers" && activeEntry === entry.id}
                inUse={entry.inUse}
                onClick={() => {
                  try {
                    localStorage.setItem(STORAGE_KEY, entry.id);
                  } catch {
                    // The choice is only remembered when storage works.
                  }
                  onSelectEntry(entry.id);
                }}
                icon={
                  entry.icon ? (
                    <ProviderIcon
                      icon={entry.icon}
                      name={entry.name}
                      size={17}
                    />
                  ) : (
                    <MoreHorizontal className="h-4 w-4" />
                  )
                }
              >
                {entry.name}
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
