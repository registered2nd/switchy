import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { useUsageSummary } from "@/lib/query/usage";
import { Loader2 } from "lucide-react";
import { motion } from "framer-motion";
import { fmtUsd, parseFiniteNumber } from "./format";

interface UsageSummaryCardsProps {
  days: number;
  refreshIntervalMs: number;
}

export function UsageSummaryCards({
  days,
  refreshIntervalMs,
}: UsageSummaryCardsProps) {
  const { t } = useTranslation();

  const { data: summary, isLoading } = useUsageSummary(days, {
    refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
  });

  const stats = useMemo(() => {
    const totalRequests = summary?.totalRequests ?? 0;
    const totalCost = parseFiniteNumber(summary?.totalCost);

    const inputTokens = summary?.totalInputTokens ?? 0;
    const outputTokens = summary?.totalOutputTokens ?? 0;
    const totalTokens = inputTokens + outputTokens;

    const cacheWriteTokens = summary?.totalCacheCreationTokens ?? 0;
    const cacheReadTokens = summary?.totalCacheReadTokens ?? 0;
    const totalCacheTokens = cacheWriteTokens + cacheReadTokens;

    return [
      {
        title: t("usage.totalRequests"),
        value: totalRequests.toLocaleString(),
        subValue: null,
      },
      {
        title: t("usage.totalCost"),
        value: totalCost == null ? "--" : fmtUsd(totalCost, 4),
        subValue: null,
      },
      {
        title: t("usage.totalTokens"),
        value: totalTokens.toLocaleString(),
        subValue: (
          <div className="flex flex-col gap-1 text-xs text-muted-foreground mt-3 pt-3 border-t border-border/50">
            <div className="flex justify-between items-center">
              <span>{t("usage.input")}</span>
              <span className="text-foreground/80">
                {(inputTokens / 1000).toFixed(1)}k
              </span>
            </div>
            <div className="flex justify-between items-center">
              <span>{t("usage.output")}</span>
              <span className="text-foreground/80">
                {(outputTokens / 1000).toFixed(1)}k
              </span>
            </div>
          </div>
        ),
      },
      {
        title: t("usage.cacheTokens"),
        value: totalCacheTokens.toLocaleString(),
        subValue: (
          <div className="flex flex-col gap-1 text-xs text-muted-foreground mt-3 pt-3 border-t border-border/50">
            <div className="flex justify-between items-center">
              <span>{t("usage.cacheWrite")}</span>
              <span className="text-foreground/80">
                {(cacheWriteTokens / 1000).toFixed(1)}k
              </span>
            </div>
            <div className="flex justify-between items-center">
              <span>{t("usage.cacheRead")}</span>
              <span className="text-foreground/80">
                {(cacheReadTokens / 1000).toFixed(1)}k
              </span>
            </div>
          </div>
        ),
      },
    ];
  }, [summary, t]);

  const container = {
    hidden: { opacity: 0 },
    show: {
      opacity: 1,
      transition: {
        staggerChildren: 0.1,
      },
    },
  };

  const item = {
    hidden: { opacity: 0, y: 20 },
    show: { opacity: 1, y: 0 },
  };

  if (isLoading) {
    return (
      <div className="grid gap-4 md:grid-cols-4">
        {[...Array(4)].map((_, i) => (
          <Card
            key={i}
            className="rounded-lg border border-border bg-card shadow-none"
          >
            <CardContent className="p-6 flex items-center justify-center min-h-[160px]">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground/50" />
            </CardContent>
          </Card>
        ))}
      </div>
    );
  }

  return (
    <motion.div
      variants={container}
      initial="hidden"
      animate="show"
      className="grid gap-4 md:grid-cols-4"
    >
      {stats.map((stat, i) => (
        <motion.div key={i} variants={item}>
          <Card className="relative h-full overflow-hidden rounded-lg border border-border bg-card shadow-none">
            <CardContent className="p-4">
              <p className="mb-1 text-[12.5px] text-muted-foreground">
                {stat.title}
              </p>

              <div className="space-y-1">
                <h3
                  className="truncate font-display text-[26px] font-semibold tabular-nums tracking-tight"
                  title={stat.value}
                >
                  {stat.value}
                </h3>
              </div>

              {stat.subValue || (
                /* Placeholder to properly align cards if no subvalue (first 2 cards) - effectively adding empty space or using flex-1 equivalent */
                <div className="mt-3 pt-3 border-t border-transparent h-[52px]"></div>
              )}
            </CardContent>
          </Card>
        </motion.div>
      ))}
    </motion.div>
  );
}
