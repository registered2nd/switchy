import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plus } from "lucide-react";
import { toast } from "sonner";
import type { Provider } from "@/types";
import { providersApi, type AppId } from "@/lib/api";
import { accountsApi, type Account } from "@/lib/api/accounts";
import { useAccountsQuery } from "@/lib/query/accounts";
import { useProvidersQuery } from "@/lib/query";
import { useProviderActions } from "@/hooks/useProviderActions";
import { extractErrorMessage } from "@/utils/errorUtils";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import {
  companyById,
  isOwnSignIn,
  OTHER_COMPANY,
  PROGRAM_NAMES,
  relayPresetNames,
  sidebarEntryOf,
  type Company,
  type SidebarEntryId,
} from "@/config/companies";
import { ProgramAccounts } from "./ProgramAccounts";
import { PlanAccountCard, PlanUsage } from "./PlanAccountCard";
import { MonitorCard } from "./MonitorCard";
import { AccountDialog } from "./AccountDialog";
import { UsedInChips } from "./UsedInChips";
import { usePlanUse } from "./usePlanUse";

/** Programs a company's page lists, in order. */
function programsOf(company: Company): AppId[] {
  const apps: AppId[] = [];
  for (const app of [...(company.logins ?? []), ...(company.relayApps ?? [])]) {
    if (!apps.includes(app)) apps.push(app);
  }
  return apps;
}

/** The Kimi plan key filed with a Kimi Code sign-in, and where the key is used. */
function KimiKeyUse({ account }: { account: Account }) {
  const { choices, region, runIn, stopIn } = usePlanUse(account);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState<AppId | null>(null);
  const { t } = useTranslation();
  const company = companyById("kimi")!;
  return (
    <div className="flex flex-wrap items-center gap-3">
      <PlanUsage
        account={account}
        quotaUrl={region?.quotaUrl}
        consoleUrl={region?.consoleUrl}
      />
      <UsedInChips
        chips={choices.map(({ app, inUse }) => ({ app, inUse }))}
        busy={busy}
        onPick={async (app, inUse) => {
          setBusy(app);
          try {
            if (inUse) await stopIn(app);
            else await runIn(app);
          } finally {
            setBusy(null);
          }
        }}
      />
      <button
        type="button"
        className="text-[12px] text-muted-foreground hover:text-foreground hover:underline"
        onClick={() => setEditing(true)}
      >
        {t("account.editConsoleKey", { defaultValue: "Console key" })}
      </button>
      <AccountDialog
        open={editing}
        onOpenChange={setEditing}
        company={company}
        kind="plan"
        account={account}
      />
    </div>
  );
}

/** Under a Kimi Code sign-in: its plan key's programs, or a way to add the key. */
function KimiSignInExtra({
  provider,
  accounts,
}: {
  provider: Provider;
  accounts: Account[];
}) {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const account = accounts.find(
    (a) =>
      a.company === "kimi" &&
      a.kind === "plan" &&
      a.loginProviderId === provider.id,
  );
  if (account) return <KimiKeyUse account={account} />;
  return (
    <>
      <button
        type="button"
        className="flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
        onClick={() => setAdding(true)}
      >
        <KeyRound className="h-3 w-3" />
        {t("account.addConsoleKey", {
          defaultValue:
            "Add its console key to use the plan in Claude Code or OpenCode",
        })}
      </button>
      <AccountDialog
        open={adding}
        onOpenChange={setAdding}
        company={companyById("kimi")!}
        kind="plan"
        loginProviderId={provider.id}
        defaultName={provider.name}
      />
    </>
  );
}

/** Adds a relay or gateway account to one program. */
function RelayDialog({
  app,
  open,
  onOpenChange,
}: {
  app: AppId;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const { addProvider } = useProviderActions(app);
  return (
    <AddProviderDialog
      open={open}
      onOpenChange={onOpenChange}
      appId={app}
      onSubmit={addProvider}
      presetNames={relayPresetNames(app)}
      title={t("account.addRelayTitle", {
        program: PROGRAM_NAMES[app],
        defaultValue: "Add a relay or gateway for {{program}}",
      })}
    />
  );
}

/** The page header's Add menu: every kind of account the company holds. */
export function AddAccountMenu({
  entry,
  onOpenUniversal,
}: {
  entry: SidebarEntryId;
  /** Opens the page of relays shared by Claude Code, Codex and Gemini CLI. */
  onOpenUniversal?: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const company = companyById(entry);
  const [relayApp, setRelayApp] = useState<AppId | null>(null);
  const [accountKind, setAccountKind] = useState<"plan" | "monitor" | null>(
    null,
  );
  if (!company) return null;

  const addSignIn = async (app: AppId) => {
    try {
      await accountsApi.addSignIn(app);
      await queryClient.invalidateQueries({ queryKey: ["providers", app] });
      await providersApi.updateTrayMenu();
      toast.success(
        t("account.signInAdded", {
          program: PROGRAM_NAMES[app],
          defaultValue:
            "Added. Use it in {{program}}, then sign in there; the sign-in is filed with this card.",
        }),
        { closeButton: true, duration: 8000 },
      );
    } catch (error) {
      toast.error(extractErrorMessage(error));
    }
  };

  const importOpenCode = async () => {
    try {
      const count = await providersApi.importOpenCodeFromLive();
      await queryClient.invalidateQueries({
        queryKey: ["providers", "opencode"],
      });
      await queryClient.invalidateQueries({
        queryKey: ["opencodeLiveProviderIds"],
      });
      if (count > 0) toast.success(t("provider.importCurrentDescription"));
      else toast.info(t("provider.noProviders"));
    } catch (error) {
      toast.error(extractErrorMessage(error));
    }
  };

  const importSignIn = async (app: AppId) => {
    try {
      const imported = await providersApi.importDefault(app);
      await queryClient.invalidateQueries({ queryKey: ["providers", app] });
      if (imported) toast.success(t("provider.importCurrentDescription"));
      else toast.info(t("provider.noProviders"));
    } catch (error) {
      toast.error(extractErrorMessage(error));
    }
  };

  const signInLabel = (app: AppId) =>
    t(`account.signInKind.${app}`, {
      defaultValue:
        {
          claude: "Claude subscription (Claude Code)",
          codex: "ChatGPT subscription (Codex)",
          gemini: "Google sign-in (Gemini CLI)",
          antigravity: "Google AI subscription (Antigravity)",
          kimi: "Kimi Code sign-in",
          cursor: "Cursor subscription (Cursor CLI)",
        }[app as string] ?? PROGRAM_NAMES[app],
    });

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" className="gap-1.5">
            <Plus className="h-4 w-4" />
            {t("account.add", { defaultValue: "Add account" })}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-72">
          {(company.logins ?? []).map((app) => (
            <DropdownMenuItem
              key={`sign-in-${app}`}
              onClick={() => void addSignIn(app)}
            >
              {signInLabel(app)}
            </DropdownMenuItem>
          ))}
          {(company.plans?.length ?? 0) > 0 && company.id !== "kimi" && (
            <DropdownMenuItem onClick={() => setAccountKind("plan")}>
              {t("account.addPlan", {
                plan: company.plans!.map((p) => p.name).join(" / "),
                defaultValue: "{{plan}} account",
              })}
            </DropdownMenuItem>
          )}
          {(company.relayApps ?? []).length > 0 && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">
                {t("account.relays", { defaultValue: "Relays and gateways" })}
              </DropdownMenuLabel>
              {company.relayApps!.map((app) => (
                <DropdownMenuItem
                  key={`relay-${app}`}
                  onClick={() => setRelayApp(app)}
                >
                  {t("account.relayFor", {
                    program: PROGRAM_NAMES[app],
                    defaultValue: "For {{program}}…",
                  })}
                </DropdownMenuItem>
              ))}
              {company.relayApps!.includes("opencode") && (
                <DropdownMenuItem onClick={() => void importOpenCode()}>
                  {t("account.importOpenCode", {
                    defaultValue: "Import the providers in OpenCode's config",
                  })}
                </DropdownMenuItem>
              )}
              {onOpenUniversal &&
                company.relayApps!.some((app) =>
                  ["claude", "codex", "gemini"].includes(app),
                ) && (
                  <DropdownMenuItem onClick={onOpenUniversal}>
                    {t("account.universalRelay", {
                      defaultValue:
                        "One relay for Claude Code, Codex and Gemini CLI…",
                    })}
                  </DropdownMenuItem>
                )}
            </>
          )}
          {(company.monitorRegions?.length ?? 0) > 0 && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => setAccountKind("monitor")}>
                {t("account.addMonitorItem", {
                  defaultValue: "Pay-as-you-go balance monitor",
                })}
              </DropdownMenuItem>
            </>
          )}
          {(company.logins ?? []).length > 0 && (
            <>
              <DropdownMenuSeparator />
              {company.logins!.map((app) => (
                <DropdownMenuItem
                  key={`import-${app}`}
                  onClick={() => void importSignIn(app)}
                >
                  {t("account.importSignIn", {
                    program: PROGRAM_NAMES[app],
                    defaultValue:
                      "Import the {{program}} account signed in now",
                  })}
                </DropdownMenuItem>
              ))}
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      {relayApp && (
        <RelayDialog
          app={relayApp}
          open={Boolean(relayApp)}
          onOpenChange={(open) => !open && setRelayApp(null)}
        />
      )}
      <AccountDialog
        open={accountKind !== null}
        onOpenChange={(open) => !open && setAccountKind(null)}
        company={company}
        kind={accountKind ?? "plan"}
      />
    </>
  );
}

/** One sidebar entry's page: a company's accounts, or the cards no company lists. */
export function CompanyView({ entry }: { entry: SidebarEntryId }) {
  const { t } = useTranslation();
  const company = companyById(entry);
  const { data: accounts = [] } = useAccountsQuery();
  const apps: AppId[] = company
    ? programsOf(company)
    : ["claude", "codex", "gemini", "kimi", "opencode"];

  // The empty state needs to know whether any program lists a card here.
  const claude = useProvidersQuery("claude");
  const codex = useProvidersQuery("codex");
  const gemini = useProvidersQuery("gemini");
  const kimi = useProvidersQuery("kimi");
  const cursor = useProvidersQuery("cursor");
  const antigravity = useProvidersQuery("antigravity");
  const opencode = useProvidersQuery("opencode");
  const byApp: Partial<Record<AppId, Record<string, Provider> | undefined>> = {
    claude: claude.data?.providers,
    codex: codex.data?.providers,
    gemini: gemini.data?.providers,
    kimi: kimi.data?.providers,
    cursor: cursor.data?.providers,
    antigravity: antigravity.data?.providers,
    opencode: opencode.data?.providers,
  };

  const filters = useMemo(() => {
    const map: Partial<Record<AppId, (p: Provider) => boolean>> = {};
    for (const app of apps) {
      map[app] = (provider: Provider) =>
        sidebarEntryOf(app, provider) === entry;
    }
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry, apps.join(",")]);

  const kimiExtra = useCallback(
    (provider: Provider) =>
      isOwnSignIn("kimi", provider) ? (
        <KimiSignInExtra provider={provider} accounts={accounts} />
      ) : null,
    [accounts],
  );

  const plans = accounts.filter(
    (a) => a.company === entry && a.kind === "plan" && !a.loginProviderId,
  );
  const monitors = accounts.filter(
    (a) => a.company === entry && a.kind === "monitor",
  );
  const hasCards = apps.some((app) =>
    Object.values(byApp[app] ?? {}).some((p) => filters[app]?.(p)),
  );

  return (
    <div className="space-y-4">
      {entry === OTHER_COMPANY && (
        <p className="text-[12.5px] text-muted-foreground">
          {t("account.otherHint", {
            defaultValue:
              "Pay-as-you-go keys set up in a program. Switchy runs subscription accounts and only monitors pay-as-you-go balances: add a balance monitor under the company to see what is left and what was spent.",
          })}
        </p>
      )}

      {apps.map((app) => (
        <ProgramAccounts
          key={app}
          appId={app}
          filter={filters[app]!}
          renderExtraUsedIn={
            app === "kimi" && entry === "kimi" ? kimiExtra : undefined
          }
        />
      ))}

      {plans.length > 0 && (
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
          {plans.map((account) => (
            <PlanAccountCard key={account.id} account={account} />
          ))}
        </div>
      )}

      {monitors.length > 0 && (
        <section className="space-y-2">
          <h2 className="text-[12.5px] font-medium text-muted-foreground">
            {t("account.monitorsHeading", {
              defaultValue: "Pay-as-you-go balance (monitor only)",
            })}
          </h2>
          <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
            {monitors.map((account) => (
              <MonitorCard key={account.id} account={account} />
            ))}
          </div>
        </section>
      )}

      {!hasCards && plans.length === 0 && monitors.length === 0 && company && (
        <div className="rounded-lg border border-dashed border-border px-6 py-10 text-center text-sm text-muted-foreground">
          {t("account.empty", {
            company: company.name,
            defaultValue:
              "No {{company}} accounts yet. Add one with Add account.",
          })}
        </div>
      )}
    </div>
  );
}
