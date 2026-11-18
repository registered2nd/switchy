import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";
import { useRelayBalanceQuery } from "@/lib/query/accounts";

const dollars = (amount: number) =>
  new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  }).format(amount);

const units = (amount: number) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(amount);

/**
 * A relay card's balance in dollars, read with the card's key. A card whose
 * relay has no balance call shows nothing.
 */
export function RelayBalanceBadge({
  provider,
  appId,
}: {
  provider: Provider;
  appId: AppId;
}) {
  const { t } = useTranslation();
  const { data: report } = useRelayBalanceQuery(provider, appId);
  if (!report) return null;

  if (!report.success || report.balance === null) {
    return (
      <span
        className="max-w-[240px] truncate text-[12px] text-muted-foreground"
        title={report.error ?? undefined}
      >
        {report.error}
      </span>
    );
  }

  const title =
    report.units !== null && report.unitsPerDollar !== 1
      ? t("relayBalance.units", {
          units: units(report.units),
          rate: units(report.unitsPerDollar),
          defaultValue:
            "{{units}} of the relay's units, {{rate}} to the dollar",
        })
      : undefined;

  return (
    <div className="text-right text-[12px]" title={title}>
      <div className="text-muted-foreground">
        {t("account.balance", { defaultValue: "Balance" })}
      </div>
      <div className="text-[14px] font-medium tabular-nums">
        {dollars(report.balance)}
      </div>
    </div>
  );
}
