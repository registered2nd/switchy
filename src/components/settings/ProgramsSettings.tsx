import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import { useProvidersQuery } from "@/lib/query";
import { useAccountsQuery } from "@/lib/query/accounts";
import type { SettingsFormState } from "@/hooks/useSettings";
import { useProviderActions } from "@/hooks/useProviderActions";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ProxyToggle } from "@/components/proxy/ProxyToggle";
import { FailoverToggle } from "@/components/proxy/FailoverToggle";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import { ProgramAccounts } from "@/components/companies/ProgramAccounts";
import {
  companyById,
  PROGRAM_NAMES,
  sidebarEntryOf,
  OTHER_COMPANY,
} from "@/config/companies";

const PROGRAM_ICON: Record<AppId, string> = {
  claude: "claude",
  codex: "openai",
  gemini: "gemini",
  kimi: "kimi",
  cursor: "cursor",
  antigravity: "google",
  opencode: "opencode",
  openclaw: "openclaw",
};

const SWITCHED: AppId[] = [
  "claude",
  "codex",
  "gemini",
  "kimi",
  "cursor",
  "antigravity",
];

/** The account a program runs on now, named the way its company page names it. */
function InUse({ app }: { app: AppId }) {
  const { t } = useTranslation();
  const { data } = useProvidersQuery(app);
  const { data: accounts = [] } = useAccountsQuery();
  const current: Provider | undefined =
    data?.providers?.[data.currentProviderId];
  if (!current) {
    return (
      <span className="text-muted-foreground">
        {t("programs.noneInUse", { defaultValue: "No account in use" })}
      </span>
    );
  }
  const account = current.meta?.accountId
    ? accounts.find((a) => a.id === current.meta?.accountId)
    : undefined;
  const entry = account ? account.company : sidebarEntryOf(app, current);
  const company =
    entry && entry !== OTHER_COMPANY ? companyById(entry)?.name : undefined;
  const name =
    account?.name ??
    current.meta?.capturedClaudeAccount?.emailAddress ??
    current.name;
  return (
    <span className="truncate">{company ? `${company} · ${name}` : name}</span>
  );
}

function OmoProfiles() {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const { addProvider } = useProviderActions("opencode");
  return (
    <div className="space-y-2 pl-8">
      <div className="flex items-center justify-between">
        <span className="text-[12.5px] text-muted-foreground">
          {t("programs.omoProfiles", {
            defaultValue: "Oh My OpenCode profiles",
          })}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="gap-1"
          onClick={() => setAdding(true)}
        >
          <Plus className="h-3.5 w-3.5" />
          {t("common.add")}
        </Button>
      </div>
      <ProgramAccounts
        appId="opencode"
        showProgram={false}
        filter={(p) => p.category === "omo" || p.category === "omo-slim"}
      />
      <AddProviderDialog
        open={adding}
        onOpenChange={setAdding}
        appId="opencode"
        onSubmit={addProvider}
        presetNames={["Oh My OpenCode", "Oh My OpenCode Slim"]}
      />
    </div>
  );
}

interface ProgramsSettingsProps {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
  onOpenOpenClaw?: () => void;
}

/**
 * The programs Switchy writes accounts into: which account each runs on,
 * whether its requests go through the local proxy, and the program settings
 * that are not accounts.
 */
export function ProgramsSettings({
  settings,
  onChange,
  onOpenOpenClaw,
}: ProgramsSettingsProps) {
  const { t } = useTranslation();
  const visibleApps = settings.visibleApps;
  const openclawShown = Boolean(visibleApps?.openclaw);

  const row = (
    app: AppId,
    content: React.ReactNode,
    controls?: React.ReactNode,
  ) => (
    <div key={app} className="flex items-center gap-3 px-4 py-3">
      <ProviderIcon
        icon={PROGRAM_ICON[app]}
        name={PROGRAM_NAMES[app]}
        size={20}
      />
      <div className="min-w-0 flex-1">
        <div className="text-[14px] font-medium">{PROGRAM_NAMES[app]}</div>
        <div className="truncate text-[12px]">{content}</div>
      </div>
      <div className="flex shrink-0 items-center gap-2">{controls}</div>
    </div>
  );

  return (
    <div className="space-y-6">
      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
        {SWITCHED.map((app) =>
          row(
            app,
            <InUse app={app} />,
            app === "cursor" || app === "antigravity" ? null : (
              <>
                {settings.enableLocalProxy && <ProxyToggle activeApp={app} />}
                {settings.enableFailoverToggle && (
                  <FailoverToggle activeApp={app} />
                )}
              </>
            ),
          ),
        )}
        <div>
          {row(
            "opencode",
            <span className="text-muted-foreground">
              {t("programs.opencodeHint", {
                defaultValue:
                  "Holds several accounts at once; each card's Used in adds or removes it.",
              })}
            </span>,
          )}
          <div className="pb-3">
            <OmoProfiles />
          </div>
        </div>
        {row(
          "openclaw",
          <span className="text-muted-foreground">
            {t("programs.openclawHint", {
              defaultValue: "A gateway with its own providers, off by default.",
            })}
          </span>,
          <>
            {openclawShown && onOpenOpenClaw && (
              <Button variant="outline" size="sm" onClick={onOpenOpenClaw}>
                {t("programs.open", { defaultValue: "Open" })}
              </Button>
            )}
            <Switch
              checked={openclawShown}
              onCheckedChange={(checked) =>
                onChange({
                  visibleApps: {
                    claude: true,
                    codex: true,
                    gemini: true,
                    kimi: true,
                    cursor: true,
                    antigravity: true,
                    opencode: true,
                    ...(visibleApps ?? {}),
                    openclaw: checked,
                  },
                })
              }
            />
          </>,
        )}
      </div>
    </div>
  );
}
