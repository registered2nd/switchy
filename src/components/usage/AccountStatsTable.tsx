import { useTranslation } from "react-i18next";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useProviderStats } from "@/lib/query/usage";
import { fmtInt, fmtUsd, getLocaleFromLanguage } from "./format";
import { formatAgo } from "./SwitchHistoryTable";

interface AccountStatsTableProps {
  days: number;
  refreshIntervalMs: number;
}

/** Requests, tokens and refusals per account over the selected range. */
export function AccountStatsTable({
  days,
  refreshIntervalMs,
}: AccountStatsTableProps) {
  const { t, i18n } = useTranslation();
  const locale = getLocaleFromLanguage(i18n.language);
  const { data: stats, isLoading } = useProviderStats(days, {
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
            <TableHead>{t("usage.account")}</TableHead>
            <TableHead className="text-right">{t("usage.requests")}</TableHead>
            <TableHead className="text-right">{t("usage.tokens")}</TableHead>
            <TableHead className="text-right">
              {t("usage.successRate")}
            </TableHead>
            <TableHead className="text-right">{t("usage.refused")}</TableHead>
            <TableHead className="text-right">{t("usage.lastUsed")}</TableHead>
            <TableHead className="text-right">
              {t("usage.avgLatency")}
            </TableHead>
            <TableHead className="text-right">{t("usage.cost")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {!stats || stats.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={8}
                className="text-center text-muted-foreground"
              >
                {t("usage.noData")}
              </TableCell>
            </TableRow>
          ) : (
            stats.map((stat) => (
              <TableRow key={`${stat.appType}:${stat.providerId}`}>
                <TableCell>
                  <div className="flex flex-col">
                    <span className="font-medium">
                      {stat.accountEmail || stat.providerName}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t(`apps.${stat.appType}`, {
                        defaultValue: stat.appType,
                      })}
                      {stat.accountEmail ? ` · ${stat.providerName}` : ""}
                    </span>
                  </div>
                </TableCell>
                <TableCell className="text-right">
                  {fmtInt(stat.requestCount, locale)}
                </TableCell>
                <TableCell className="text-right">
                  {fmtInt(stat.totalTokens, locale)}
                </TableCell>
                <TableCell className="text-right">
                  {stat.successRate.toFixed(1)}%
                </TableCell>
                <TableCell
                  className={
                    stat.limitedCount > 0
                      ? "text-right text-amber-600 dark:text-amber-400"
                      : "text-right"
                  }
                >
                  {fmtInt(stat.limitedCount, locale)}
                </TableCell>
                <TableCell className="text-right text-muted-foreground">
                  {stat.lastUsedAt ? formatAgo(stat.lastUsedAt, t) : "--"}
                </TableCell>
                <TableCell className="text-right">
                  {stat.avgLatencyMs}ms
                </TableCell>
                <TableCell className="text-right text-muted-foreground">
                  {fmtUsd(stat.totalCost, 4)}
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}
