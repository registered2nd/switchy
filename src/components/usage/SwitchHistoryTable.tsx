import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAccountSwitches } from "@/lib/query/usage";
import type { SwitchReason } from "@/types/usage";
import { cn } from "@/lib/utils";

/** "just now", "5 min ago", ... from Unix seconds. */
export function formatAgo(unixSeconds: number, t: TFunction): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (diff < 60) return t("usage.justNow");
  if (diff < 3600)
    return t("usage.minutesAgo", { count: Math.floor(diff / 60) });
  if (diff < 86400)
    return t("usage.hoursAgo", { count: Math.floor(diff / 3600) });
  return t("usage.daysAgo", { count: Math.floor(diff / 86400) });
}

const REASON_STYLE: Record<SwitchReason, string> = {
  manual: "bg-blue-500/10 text-blue-600 dark:text-blue-400",
  failover: "bg-orange-500/10 text-orange-600 dark:text-orange-400",
  limit: "bg-amber-500/10 text-amber-600 dark:text-amber-400",
  signed_out: "bg-red-500/10 text-red-600 dark:text-red-400",
  rotation: "bg-violet-500/10 text-violet-600 dark:text-violet-400",
  recovered: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
};

interface SwitchHistoryTableProps {
  days: number;
  refreshIntervalMs: number;
}

/** Every change of the account serving an app, and why it happened. */
export function SwitchHistoryTable({
  days,
  refreshIntervalMs,
}: SwitchHistoryTableProps) {
  const { t, i18n } = useTranslation();
  const { data: switches, isLoading } = useAccountSwitches(days, {
    refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
  });

  if (isLoading) {
    return <div className="h-[400px] animate-pulse rounded bg-muted/40" />;
  }

  return (
    <div className="rounded-lg border border-border/50 bg-card/40 backdrop-blur-sm overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("usage.time")}</TableHead>
            <TableHead>{t("usage.appType")}</TableHead>
            <TableHead>{t("usage.switchedAccounts")}</TableHead>
            <TableHead>{t("usage.switchReason")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {!switches || switches.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={4}
                className="text-center text-muted-foreground"
              >
                {t("usage.noSwitches")}
              </TableCell>
            </TableRow>
          ) : (
            switches.map((s) => (
              <TableRow key={s.id}>
                <TableCell
                  className="whitespace-nowrap text-muted-foreground"
                  title={new Date(s.createdAt * 1000).toLocaleString(
                    i18n.language,
                  )}
                >
                  {formatAgo(s.createdAt, t)}
                </TableCell>
                <TableCell>
                  {t(`apps.${s.appType}`, { defaultValue: s.appType })}
                </TableCell>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <span className="text-muted-foreground">
                      {s.fromAccount ??
                        s.fromProviderName ??
                        s.fromProviderId ??
                        "--"}
                    </span>
                    <ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
                    <span className="font-medium">
                      {s.toAccount ?? s.toProviderName ?? s.toProviderId}
                    </span>
                  </div>
                </TableCell>
                <TableCell>
                  <div className="flex flex-col gap-1">
                    <span
                      className={cn(
                        "w-fit rounded-full px-2 py-0.5 text-xs font-medium",
                        REASON_STYLE[s.reason],
                      )}
                    >
                      {t(`usage.switchReasons.${s.reason}`)}
                    </span>
                    {s.detail && (
                      <span className="text-xs text-muted-foreground">
                        {s.detail}
                      </span>
                    )}
                  </div>
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}
