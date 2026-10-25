import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Edit,
  ExternalLink,
  Loader2,
  MoreHorizontal,
  Trash2,
} from "lucide-react";
import type { AppId } from "@/lib/api";
import { settingsApi } from "@/lib/api";
import type { Account } from "@/lib/api/accounts";
import { usePlanQuotaQuery } from "@/lib/query/accounts";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ProviderIcon } from "@/components/ProviderIcon";
import { TierMeter } from "@/components/SubscriptionQuotaFooter";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { planOf, REGION_NAMES, type RegionId } from "@/config/companies";
import { UsedInChips } from "./UsedInChips";
import { AccountDialog } from "./AccountDialog";
import { usePlanUse } from "./usePlanUse";

/** The plan's usage, read with the account's key the way the company records it. */
export function PlanUsage({
  account,
  quotaUrl,
  consoleUrl,
}: {
  account: Account;
  quotaUrl?: string;
  consoleUrl?: string;
}) {
  const { t } = useTranslation();
  const { data: quota, isFetching } = usePlanQuotaQuery(
    account,
    Boolean(quotaUrl),
  );

  if (!quotaUrl) {
    return consoleUrl ? (
      <button
        type="button"
        className="text-[12px] text-muted-foreground hover:text-foreground hover:underline"
        onClick={() => void settingsApi.openExternal(consoleUrl)}
      >
        {t("account.usageInConsole", { defaultValue: "Usage in the console" })}
      </button>
    ) : null;
  }
  if (!quota) {
    return (
      <span className="flex items-center gap-1 text-[12px] text-muted-foreground">
        {isFetching && <Loader2 className="h-3 w-3 animate-spin" />}
        {t("usage.loading", { defaultValue: "Loading…" })}
      </span>
    );
  }
  if (!quota.success) {
    return (
      <span
        className="max-w-[260px] truncate text-[12px] text-destructive"
        title={quota.error ?? undefined}
      >
        {quota.error ??
          t("usage.queryFailed", { defaultValue: "Query failed" })}
      </span>
    );
  }
  return (
    <div
      className="flex items-start gap-3"
      title={quota.credentialMessage ?? undefined}
    >
      {quota.tiers.map((tier) => (
        <TierMeter key={tier.name} tier={tier} t={t} />
      ))}
    </div>
  );
}

/** A subscription plan reached with a key: its usage, and the programs it is used in. */
export function PlanAccountCard({ account }: { account: Account }) {
  const { t } = useTranslation();
  const {
    company,
    region,
    choices,
    runIn,
    stopIn,
    remove: removeAccount,
  } = usePlanUse(account);
  const [busy, setBusy] = useState<AppId | null>(null);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const plan = planOf(company, account.plan);
  const regionName =
    Object.keys(plan?.regions ?? {}).length > 1
      ? REGION_NAMES[(account.region as RegionId) ?? "intl"]
      : null;
  const inUse = choices.some((choice) => choice.inUse);

  const pick = async (app: AppId, isInUse: boolean) => {
    setBusy(app);
    try {
      if (isInUse) await stopIn(app);
      else await runIn(app);
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    setConfirmDelete(false);
    await removeAccount();
  };

  return (
    <div
      className={cn(
        "relative bg-card px-4 py-3 text-card-foreground transition-colors",
        inUse ? "bg-primary/[0.05]" : "hover:bg-muted/40",
      )}
    >
      {inUse && (
        <span
          aria-hidden
          className="absolute bottom-2 left-0 top-2 w-[3px] rounded-r-full bg-primary"
        />
      )}
      <div className="flex items-center gap-3">
        <ProviderIcon
          icon={company?.icon}
          name={company?.name ?? ""}
          size={22}
        />
        <div className="min-w-0 flex-1">
          <h3
            className="truncate text-[14.5px] font-medium leading-snug"
            title={account.name}
          >
            {account.name}
          </h3>
          <div className="truncate text-[12px] text-muted-foreground">
            {[plan?.name, regionName].filter(Boolean).join(" · ")}
          </div>
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-3">
          <PlanUsage
            account={account}
            quotaUrl={region?.quotaUrl}
            consoleUrl={region?.consoleUrl}
          />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => setEditing(true)}>
                <Edit className="mr-2 h-4 w-4" />
                {t("common.edit", { defaultValue: "Edit" })}
              </DropdownMenuItem>
              {region?.consoleUrl && (
                <DropdownMenuItem
                  onClick={() =>
                    void settingsApi.openExternal(region.consoleUrl)
                  }
                >
                  <ExternalLink className="mr-2 h-4 w-4" />
                  {t("account.openConsole", { defaultValue: "Open console" })}
                </DropdownMenuItem>
              )}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onClick={() => setConfirmDelete(true)}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                {t("common.delete", { defaultValue: "Delete" })}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <div className="mt-2 pl-[34px]">
        <UsedInChips
          chips={choices.map(({ app, inUse }) => ({ app, inUse }))}
          onPick={(app, isInUse) => void pick(app, isInUse)}
          busy={busy}
        />
      </div>

      {company && (
        <AccountDialog
          open={editing}
          onOpenChange={setEditing}
          company={company}
          kind="plan"
          account={account}
        />
      )}
      <ConfirmDialog
        isOpen={confirmDelete}
        title={t("account.deleteTitle", { defaultValue: "Delete account" })}
        message={t("account.deleteMessage", {
          name: account.name,
          defaultValue:
            "Delete {{name}}? The programs it is used in lose it too.",
        })}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  );
}
