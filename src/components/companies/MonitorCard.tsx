import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import {
  Edit,
  ExternalLink,
  Loader2,
  MoreHorizontal,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { accountsApi, type Account } from "@/lib/api/accounts";
import { accountKeys, useBalanceQuery } from "@/lib/query/accounts";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { companyById, REGION_NAMES, type RegionId } from "@/config/companies";
import { AccountDialog } from "./AccountDialog";

function money(amount: number | null | undefined, currency: string | null) {
  if (amount === null || amount === undefined) return "—";
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: currency ?? "USD",
      maximumFractionDigits: 2,
    }).format(amount);
  } catch {
    return `${amount.toFixed(2)} ${currency ?? ""}`.trim();
  }
}

/** A pay-as-you-go balance: what is left and what was spent. It runs nowhere. */
export function MonitorCard({ account }: { account: Account }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const company = companyById(account.company);
  const region = (account.region as RegionId) ?? "intl";
  const consoleUrl = company?.monitorConsole?.[region];
  const { data: report, isFetching } = useBalanceQuery(account);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const regionName =
    (company?.monitorRegions?.length ?? 0) > 1 ? REGION_NAMES[region] : null;

  const partName = (name: string) =>
    t(`account.balancePart.${name}`, {
      defaultValue:
        {
          granted: "granted",
          toppedUp: "topped up",
          cash: "cash",
          voucher: "voucher",
        }[name] ?? name,
    });

  const remove = async () => {
    setConfirmDelete(false);
    try {
      await accountsApi.delete(account.id);
    } catch (error) {
      toast.error(String(error));
    }
    await queryClient.invalidateQueries({ queryKey: accountKeys.all });
  };

  const since = report?.trackingSince
    ? new Date(report.trackingSince).toLocaleDateString()
    : null;

  return (
    <div className="relative bg-card px-4 py-3 text-card-foreground hover:bg-muted/40">
      <div className="flex items-center gap-3">
        <ProviderIcon
          icon={company?.icon}
          name={company?.name ?? ""}
          size={22}
        />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[14.5px] font-medium leading-snug">
            {account.name}
          </h3>
          <div className="truncate text-[12px] text-muted-foreground">
            {[
              t("account.payAsYouGo", { defaultValue: "Pay as you go" }),
              regionName,
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-4 text-[12px]">
          {!report ? (
            <span className="flex items-center gap-1 text-muted-foreground">
              {isFetching && <Loader2 className="h-3 w-3 animate-spin" />}
              {t("usage.loading", { defaultValue: "Loading…" })}
            </span>
          ) : !report.success ? (
            <span
              className="max-w-[260px] truncate text-destructive"
              title={report.error ?? undefined}
            >
              {report.error}
            </span>
          ) : (
            <>
              <div
                className="text-right"
                title={report.parts
                  .map(
                    (part) =>
                      `${partName(part.name)} ${money(part.amount, report.currency)}`,
                  )
                  .join(" · ")}
              >
                <div className="text-muted-foreground">
                  {t("account.balance", { defaultValue: "Balance" })}
                </div>
                <div className="text-[14px] font-medium tabular-nums">
                  {money(report.balance, report.currency)}
                </div>
              </div>
              <div
                className="text-right"
                title={
                  since
                    ? t("account.spendSince", {
                        date: since,
                        defaultValue:
                          "Counted from the balance readings since {{date}}",
                      })
                    : undefined
                }
              >
                <div className="text-muted-foreground">
                  {t("account.spent", { defaultValue: "Spent today / month" })}
                </div>
                <div className="font-medium tabular-nums">
                  {report.spentToday === null
                    ? t("account.spendPending", {
                        defaultValue: "after the next reading",
                      })
                    : `${money(report.spentToday, report.currency)} / ${money(
                        report.spentMonth,
                        report.currency,
                      )}`}
                </div>
              </div>
            </>
          )}
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
              {consoleUrl && (
                <DropdownMenuItem
                  onClick={() => void settingsApi.openExternal(consoleUrl)}
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

      {company && (
        <AccountDialog
          open={editing}
          onOpenChange={setEditing}
          company={company}
          kind="monitor"
          account={account}
        />
      )}
      <ConfirmDialog
        isOpen={confirmDelete}
        title={t("account.deleteMonitorTitle", {
          defaultValue: "Delete balance monitor",
        })}
        message={t("account.deleteMonitorMessage", {
          name: account.name,
          defaultValue: "Stop monitoring {{name}}?",
        })}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmDelete(false)}
      />
    </div>
  );
}
