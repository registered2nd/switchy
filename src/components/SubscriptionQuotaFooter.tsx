import React from "react";
import { RefreshCw, AlertCircle, Clock } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import {
  useSubscriptionQuota,
  useSubscriptionQuotaForProvider,
} from "@/lib/query/subscription";
import type { QuotaTier } from "@/types/subscription";

interface SubscriptionQuotaFooterProps {
  appId: AppId;
  /**
   * When supplied, reads that provider's own login instead of the live
   * credentials — the captured snapshot under `~/.switchy/accounts/{id}/`
   * for Claude, the stored `auth` for Codex — so each Official card shows its
   * own account's quota rather than the current one's.
   */
  providerId?: string;
  inline?: boolean;
}

/** Display names for known tiers (shared by official subscriptions and Token Plan) */
export const TIER_I18N_KEYS: Record<string, string> = {
  five_hour: "subscription.fiveHour",
  seven_day: "subscription.sevenDay",
  seven_day_opus: "subscription.sevenDayOpus",
  seven_day_sonnet: "subscription.sevenDaySonnet",
  // Gemini model classes
  gemini_pro: "subscription.geminiPro",
  gemini_flash: "subscription.geminiFlash",
  gemini_flash_lite: "subscription.geminiFlashLite",
  // Token Plan (five_hour is already in the official map above)
  weekly_limit: "subscription.weeklyLimit",
};

/** Color class for a usage percentage */
export function utilizationColor(utilization: number): string {
  if (utilization >= 90) return "text-red-500 dark:text-red-400";
  if (utilization >= 70) return "text-orange-500 dark:text-orange-400";
  return "text-green-600 dark:text-green-400";
}

/** Plain countdown string, e.g. "2h30m" or "3d12h" */
export function countdownStr(resetsAt: string | null): string | null {
  if (!resetsAt) return null;
  const diffMs = new Date(resetsAt).getTime() - Date.now();
  if (diffMs <= 0) return null;

  const hours = Math.floor(diffMs / (1000 * 60 * 60));
  const minutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));

  if (hours > 24) {
    const days = Math.floor(hours / 24);
    return `${days}d${hours % 24}h`;
  }
  if (hours > 0) return `${hours}h${minutes}m`;
  return `${minutes}m`;
}

/** Reset time as countdown text (i18n template) */
function formatResetTime(
  resetsAt: string | null,
  t: (key: string, options?: Record<string, string>) => string,
): string | null {
  const time = countdownStr(resetsAt);
  if (!time) return null;
  return t("subscription.resetsIn", { time });
}

/** Tiers hidden in inline mode */
const HIDDEN_INLINE_TIERS = new Set(["seven_day_sonnet"]);

/** Relative time (same as UsageFooter) */
function formatRelativeTime(
  timestamp: number,
  now: number,
  t: (key: string, options?: { count?: number }) => string,
): string {
  const diff = Math.floor((now - timestamp) / 1000);
  if (diff < 60) return t("usage.justNow");
  if (diff < 3600)
    return t("usage.minutesAgo", { count: Math.floor(diff / 60) });
  if (diff < 86400)
    return t("usage.hoursAgo", { count: Math.floor(diff / 3600) });
  return t("usage.daysAgo", { count: Math.floor(diff / 86400) });
}

const SubscriptionQuotaFooter: React.FC<SubscriptionQuotaFooterProps> = ({
  appId,
  providerId,
  inline = false,
}) => {
  const { t } = useTranslation();
  const liveQuery = useSubscriptionQuota(appId, !providerId);
  const providerQuery = useSubscriptionQuotaForProvider(
    appId,
    providerId ?? "",
    !!providerId,
  );
  const {
    data: quota,
    isFetching: loading,
    refetch,
  } = providerId ? providerQuery : liveQuery;

  // Refresh the relative time periodically
  const [now, setNow] = React.useState(Date.now());
  React.useEffect(() => {
    if (!quota?.queriedAt) return;
    const interval = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(interval);
  }, [quota?.queriedAt]);

  // No credentials: render nothing
  if (!quota || quota.credentialStatus === "not_found") return null;

  // Credential parse error: render nothing (silent)
  if (quota.credentialStatus === "parse_error") return null;

  // The provider refused the login for good: it needs signing in again
  if (quota.credentialStatus === "signed_out") {
    const hint = t(`subscription.signedOutHint.${appId}`, {
      defaultValue: t("subscription.signedOutHint.claude"),
    });
    return (
      <div
        className={
          inline
            ? "inline-flex items-center gap-1.5 text-xs rounded-lg border border-red-200 dark:border-red-900 bg-red-50 dark:bg-red-950/30 px-3 py-2 shadow-sm text-red-600 dark:text-red-400"
            : "mt-3 flex items-center gap-2 rounded-xl border border-red-200 dark:border-red-900 bg-red-50 dark:bg-red-950/30 px-4 py-3 text-xs shadow-sm text-red-600 dark:text-red-400"
        }
        title={hint}
      >
        <AlertCircle size={inline ? 12 : 14} />
        <span className="font-medium">{t("subscription.signedOut")}</span>
        <span className="opacity-80">· {hint}</span>
      </div>
    );
  }

  // Credentials expired
  if (quota.credentialStatus === "expired" && !quota.success) {
    if (inline) {
      return (
        <div className="inline-flex items-center gap-2 text-xs rounded-lg border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 shadow-sm">
          <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400">
            <AlertCircle size={12} />
            <span>{t("subscription.expired")}</span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      );
    }
    return (
      <div className="mt-3 rounded-xl border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-4 py-3 shadow-sm">
        <div className="flex items-center justify-between gap-2 text-xs">
          <div className="flex items-center gap-2 text-amber-600 dark:text-amber-400">
            <AlertCircle size={14} />
            <div>
              <span className="font-medium">{t("subscription.expired")}</span>
              <span className="ml-2 text-amber-500/70 dark:text-amber-400/70">
                {t("subscription.expiredHint", { tool: appId })}
              </span>
            </div>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-amber-100 dark:hover:bg-amber-800/30 transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>
    );
  }

  // API call failed
  if (!quota.success) {
    if (inline) {
      return (
        <div className="inline-flex items-center gap-2 text-xs rounded-lg border border-border-default bg-card px-3 py-2 shadow-sm">
          <div className="flex items-center gap-1.5 text-red-500 dark:text-red-400">
            <AlertCircle size={12} />
            <span>{t("subscription.queryFailed")}</span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      );
    }
    return (
      <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
        <div className="flex items-center justify-between gap-2 text-xs">
          <div className="flex items-center gap-2 text-red-500 dark:text-red-400">
            <AlertCircle size={14} />
            <span>{quota.error || t("subscription.queryFailed")}</span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>
    );
  }

  // Data fetched
  const tiers = quota.tiers || [];
  if (tiers.length === 0) return null;

  // ── Inline mode: compact two lines ──
  if (inline) {
    const updated = quota.queriedAt
      ? formatRelativeTime(quota.queriedAt, now, t)
      : t("usage.never", { defaultValue: "Never" });
    return (
      <div className="flex flex-shrink-0 items-center gap-5 text-xs">
        {tiers
          .filter((tier) => !HIDDEN_INLINE_TIERS.has(tier.name))
          // A limit narrower than the account's (one model's, or one the API
          // names without a window) shows once it has been used.
          .filter((tier) => TIER_I18N_KEYS[tier.name] || tier.utilization >= 1)
          .map((tier) => (
            <TierMeter key={tier.name} tier={tier} t={t} />
          ))}
        <button
          onClick={(e) => {
            e.stopPropagation();
            refetch();
          }}
          disabled={loading}
          className="self-center rounded p-1 text-muted-foreground/70 transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
          title={`${t("subscription.refresh")} (${updated})`}
        >
          <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
        </button>
      </div>
    );
  }

  // ── Expanded mode: details ──
  return (
    <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-gray-500 dark:text-gray-400 font-medium">
          {t("subscription.title", { defaultValue: "Subscription Quota" })}
        </span>
        <div className="flex items-center gap-2">
          {quota.queriedAt && (
            <span className="text-[10px] text-muted-foreground/70 flex items-center gap-1">
              <Clock size={10} />
              {formatRelativeTime(quota.queriedAt, now, t)}
            </span>
          )}
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        {tiers.map((tier) => (
          <TierBar key={tier.name} tier={tier} t={t} />
        ))}
      </div>

      {/* Extra usage */}
      {quota.extraUsage?.isEnabled && (
        <div className="mt-2 pt-2 border-t border-border-default text-xs text-gray-500 dark:text-gray-400">
          <span className="font-medium">{t("subscription.extraUsage")}: </span>
          <span className="tabular-nums">
            {quota.extraUsage.currency === "USD" ? "$" : ""}
            {(quota.extraUsage.usedCredits ?? 0).toFixed(2)}
            {quota.extraUsage.monthlyLimit != null && (
              <>
                {" "}
                / {quota.extraUsage.currency === "USD" ? "$" : ""}
                {quota.extraUsage.monthlyLimit.toFixed(2)}
              </>
            )}
          </span>
        </div>
      )}
    </div>
  );
};

/** One tier in inline mode */
export const TierBadge: React.FC<{
  tier: QuotaTier;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({ tier, t }) => {
  const label = TIER_I18N_KEYS[tier.name]
    ? t(TIER_I18N_KEYS[tier.name])
    : tier.name;
  const countdown = countdownStr(tier.resetsAt);

  return (
    <div className="flex items-center gap-0.5">
      <span className="text-gray-500 dark:text-gray-400">{label}:</span>
      <span
        className={`font-semibold tabular-nums ${utilizationColor(tier.utilization)}`}
      >
        {t("subscription.utilization", { value: Math.round(tier.utilization) })}
      </span>
      {countdown && (
        <span className="text-muted-foreground/60 ml-0.5 flex items-center gap-px">
          <Clock size={10} />
          {countdown}
        </span>
      )}
    </div>
  );
};

/** A tier's label as people read it: "7-day nimbus quill" for a model tier. */
function tierLabel(
  name: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (TIER_I18N_KEYS[name]) return t(TIER_I18N_KEYS[name]);
  const model = name.match(/^(five_hour|seven_day)_(.+)$/);
  if (model) {
    return `${t(TIER_I18N_KEYS[model[1]])} ${model[2].replace(/_/g, " ")}`;
  }
  return name.replace(/_/g, " ");
}

/** Fill color of a usage meter: quiet until the limit gets close. */
function meterColor(utilization: number): string {
  if (utilization >= 90) return "bg-destructive";
  if (utilization >= 70) return "bg-amber-500";
  return "bg-foreground/55";
}

/** One tier in inline mode: label, a thin meter, used percent and reset. */
export const TierMeter: React.FC<{
  tier: QuotaTier;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({ tier, t }) => {
  const used = Math.min(Math.max(tier.utilization, 0), 100);
  const countdown = countdownStr(tier.resetsAt);
  return (
    <div className="w-[118px]" title={formatResetTime(tier.resetsAt, t) ?? undefined}>
      <div className="flex items-baseline justify-between gap-2">
        <span className="truncate text-[11.5px] text-muted-foreground">
          {tierLabel(tier.name, t)}
        </span>
        <span
          className={`text-[12px] font-medium tabular-nums ${
            used >= 90
              ? "text-destructive"
              : used >= 70
                ? "text-amber-600 dark:text-amber-400"
                : "text-foreground"
          }`}
        >
          {Math.round(used)}%
        </span>
      </div>
      <div className="mt-1 h-[3px] w-full overflow-hidden rounded-full bg-foreground/10">
        <div
          className={`h-full rounded-full ${meterColor(used)}`}
          style={{ width: `${Math.max(used, used > 0 ? 3 : 0)}%` }}
        />
      </div>
      <div className="mt-1 h-[14px] text-[11px] tabular-nums text-muted-foreground/80">
        {countdown
          ? t("subscription.resetsInShort", {
              time: countdown,
              defaultValue: "resets in {{time}}",
            })
          : ""}
      </div>
    </div>
  );
};

/** One tier's progress bar in expanded mode */
const TierBar: React.FC<{
  tier: QuotaTier;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({ tier, t }) => {
  const label = TIER_I18N_KEYS[tier.name]
    ? t(TIER_I18N_KEYS[tier.name])
    : tier.name;
  const resetText = formatResetTime(tier.resetsAt, t);

  return (
    <div className="flex items-center gap-3 text-xs">
      <span
        className="text-gray-500 dark:text-gray-400 min-w-0 font-medium"
        style={{ width: "25%" }}
      >
        {label}
      </span>

      {/* Progress bar */}
      <div className="flex-1 h-2 bg-gray-100 dark:bg-gray-800 rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all ${
            tier.utilization >= 90
              ? "bg-red-500"
              : tier.utilization >= 70
                ? "bg-orange-500"
                : "bg-green-500"
          }`}
          style={{ width: `${Math.min(tier.utilization, 100)}%` }}
        />
      </div>

      <div
        className="flex items-center gap-2 flex-shrink-0"
        style={{ width: "30%" }}
      >
        <span
          className={`font-semibold tabular-nums ${utilizationColor(tier.utilization)}`}
        >
          {Math.round(tier.utilization)}%
        </span>
        {resetText && (
          <span
            className="text-[10px] text-muted-foreground/70 truncate"
            title={resetText}
          >
            {resetText}
          </span>
        )}
      </div>
    </div>
  );
};

export default SubscriptionQuotaFooter;
