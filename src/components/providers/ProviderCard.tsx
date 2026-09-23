import { useMemo, useState, useEffect } from "react";
import { GripVertical, ChevronDown, ChevronUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import type {
  DraggableAttributes,
  DraggableSyntheticListeners,
} from "@dnd-kit/core";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import { cn } from "@/lib/utils";
import { ProviderActions } from "@/components/providers/ProviderActions";
import { ProviderIcon } from "@/components/ProviderIcon";
import UsageFooter from "@/components/UsageFooter";
import SubscriptionQuotaFooter from "@/components/SubscriptionQuotaFooter";
import { ProviderHealthBadge } from "@/components/providers/ProviderHealthBadge";
import { FailoverPriorityBadge } from "@/components/providers/FailoverPriorityBadge";
import {
  extractCodexBaseUrl,
  extractKimiBaseUrl,
  isKimiOfficialConfig,
} from "@/utils/providerConfigUtils";
import { useCodexAccountIdentity } from "@/lib/query/codexAccount";
import { useProviderHealth } from "@/lib/query/failover";
import { useUsageQuery } from "@/lib/query/queries";

interface DragHandleProps {
  attributes: DraggableAttributes;
  listeners: DraggableSyntheticListeners;
  isDragging: boolean;
}

interface ProviderCardProps {
  provider: Provider;
  isCurrent: boolean;
  appId: AppId;
  isInConfig?: boolean; // OpenCode: whether it is already in opencode.json
  isOmo?: boolean;
  isOmoSlim?: boolean;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onRemoveFromConfig?: (provider: Provider) => void;
  onDisableOmo?: () => void;
  onDisableOmoSlim?: () => void;
  onConfigureUsage: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onDuplicate: (provider: Provider) => void;
  onTest?: (provider: Provider) => void;
  onOpenTerminal?: (provider: Provider) => void;
  isTesting?: boolean;
  isProxyRunning: boolean;
  isProxyTakeover?: boolean; // proxy takeover mode (the live config is taken over; switching is hot)
  dragHandleProps?: DragHandleProps;
  isAutoFailoverEnabled?: boolean; // whether automatic failover is on
  failoverPriority?: number; // failover priority (1 = P1, 2 = P2, ...)
  isInFailoverQueue?: boolean; // whether it is in the failover queue
  onToggleFailover?: (enabled: boolean) => void; // toggle failover queue membership
  activeProviderId?: string; // provider ID the proxy is actually using (green border in failover mode)
  // OpenClaw: default model
  isDefaultModel?: boolean;
  onSetAsDefault?: () => void;
}

/** Whether this is an official provider (no custom base URL or API key; talks to the official API directly) */
function isOfficialProvider(provider: Provider, appId: AppId): boolean {
  const config = provider.settingsConfig as Record<string, any>;
  if (appId === "claude") {
    const baseUrl = config?.env?.ANTHROPIC_BASE_URL;
    return !baseUrl || (typeof baseUrl === "string" && baseUrl.trim() === "");
  }
  if (appId === "codex") {
    // No OPENAI_API_KEY → Codex CLI's built-in OAuth (official)
    const apiKey = config?.auth?.OPENAI_API_KEY;
    return !apiKey || (typeof apiKey === "string" && apiKey.trim() === "");
  }
  if (appId === "kimi") {
    // default_model uses the kimi-code/ managed login (kimi login) → official
    return isKimiOfficialConfig(config?.config);
  }
  if (appId === "gemini") {
    // No GEMINI_API_KEY and no GOOGLE_GEMINI_BASE_URL → official Google OAuth mode
    const apiKey = config?.env?.GEMINI_API_KEY;
    const baseUrl = config?.env?.GOOGLE_GEMINI_BASE_URL;
    return (
      (!apiKey || (typeof apiKey === "string" && apiKey.trim() === "")) &&
      (!baseUrl || (typeof baseUrl === "string" && baseUrl.trim() === ""))
    );
  }
  return false;
}

const extractApiUrl = (provider: Provider, fallbackText: string) => {
  if (provider.notes?.trim()) {
    return provider.notes.trim();
  }

  if (provider.websiteUrl) {
    return provider.websiteUrl;
  }

  const config = provider.settingsConfig;

  if (config && typeof config === "object") {
    const envBase =
      (config as Record<string, any>)?.env?.ANTHROPIC_BASE_URL ||
      (config as Record<string, any>)?.env?.GOOGLE_GEMINI_BASE_URL;
    if (typeof envBase === "string" && envBase.trim()) {
      return envBase;
    }

    const baseUrl = (config as Record<string, any>)?.config;

    if (typeof baseUrl === "string" && baseUrl.includes("base_url")) {
      const extractedBaseUrl =
        "credentials" in (config as Record<string, any>)
          ? extractKimiBaseUrl(baseUrl)
          : extractCodexBaseUrl(baseUrl);
      if (extractedBaseUrl) {
        return extractedBaseUrl;
      }
    }
  }

  return fallbackText;
};

export function ProviderCard({
  provider,
  isCurrent,
  appId,
  isInConfig = true,
  isOmo = false,
  isOmoSlim = false,
  onSwitch,
  onEdit,
  onDelete,
  onRemoveFromConfig,
  onDisableOmo,
  onDisableOmoSlim,
  onConfigureUsage,
  onOpenWebsite,
  onDuplicate,
  onTest,
  onOpenTerminal,
  isTesting,
  isProxyRunning,
  isProxyTakeover = false,
  dragHandleProps,
  isAutoFailoverEnabled = false,
  failoverPriority,
  isInFailoverQueue = false,
  onToggleFailover,
  activeProviderId,
  // OpenClaw: default model
  isDefaultModel,
  onSetAsDefault,
}: ProviderCardProps) {
  const { t } = useTranslation();

  // OMO and OMO Slim share the same card behavior
  const isAnyOmo = isOmo || isOmoSlim;
  const handleDisableAnyOmo = isOmoSlim ? onDisableOmoSlim : onDisableOmo;
  const isAdditiveMode = appId === "opencode" && !isAnyOmo;

  const { data: health } = useProviderHealth(provider.id, appId);

  const fallbackUrlText = t("provider.notConfigured", {
    defaultValue: "Not configured for official website",
  });

  const displayUrl = useMemo(() => {
    return extractApiUrl(provider, fallbackUrlText);
  }, [provider, fallbackUrlText]);

  const isClickableUrl = useMemo(() => {
    if (provider.notes?.trim()) {
      return false;
    }
    if (displayUrl === fallbackUrlText) {
      return false;
    }
    return true;
  }, [provider.notes, displayUrl, fallbackUrlText]);

  const usageEnabled = provider.meta?.usage_script?.enabled ?? false;
  const isOfficial = isOfficialProvider(provider, appId);
  const { data: codexAccount } = useCodexAccountIdentity(
    provider.id,
    appId === "codex" && isOfficial,
  );
  // Non-current Official cards read their own login rather than the live one.
  // So does the current card while the proxy serves the app: the proxy
  // presents the card's login, not the one the CLI has saved.
  const quotaProviderId =
    isCurrent && !isProxyTakeover
      ? undefined
      : appId === "claude" && provider.meta?.capturedClaudeAccount
        ? provider.id
        : appId === "codex" && codexAccount
          ? provider.id
          : undefined;

  // Fetch usage to tell whether there are multiple plans
  // Additive-mode apps (OpenCode/OpenClaw): use isInConfig instead of isCurrent
  const shouldAutoQuery =
    appId === "opencode" || appId === "openclaw" ? isInConfig : isCurrent;
  const autoQueryInterval = shouldAutoQuery
    ? provider.meta?.usage_script?.autoQueryInterval || 0
    : 0;

  const { data: usage } = useUsageQuery(provider.id, appId, {
    enabled: usageEnabled,
    autoQueryInterval,
  });

  const isTokenPlan =
    provider.meta?.usage_script?.templateType === "token_plan";
  const hasMultiplePlans =
    usage?.success && usage.data && usage.data.length > 1 && !isTokenPlan;

  const [isExpanded, setIsExpanded] = useState(false);

  useEffect(() => {
    if (hasMultiplePlans) {
      setIsExpanded(true);
    }
  }, [hasMultiplePlans]);

  const handleOpenWebsite = () => {
    if (!isClickableUrl) {
      return;
    }
    onOpenWebsite(displayUrl);
  };

  // Whether this is the provider "currently in use"
  // - OMO/OMO Slim providers: use isCurrent
  // - OpenClaw: the provider owning the default model is current (blue border)
  // - OpenCode (non-OMO): no notion of "current"; returns false
  // - Failover mode: the provider the proxy is actually using (activeProviderId)
  // - Normal mode: isCurrent
  const isActiveProvider = isAnyOmo
    ? isCurrent
    : appId === "openclaw"
      ? Boolean(isDefaultModel)
      : appId === "opencode"
        ? false
        : isAutoFailoverEnabled
          ? activeProviderId === provider.id
          : isCurrent;

  const isLive = isActiveProvider || (isAdditiveMode && isInConfig);
  const hasFailures = Boolean(health && health.consecutive_failures > 0);
  const accountEmail =
    appId === "claude" && isOfficial && provider.meta?.capturedClaudeAccount
      ? provider.meta.capturedClaudeAccount.emailAddress
      : appId === "codex" && codexAccount
        ? (codexAccount.email ?? codexAccount.accountId ?? "")
        : "";

  return (
    <div
      className={cn(
        "group relative bg-card px-4 py-3 text-card-foreground transition-colors",
        isLive ? "bg-primary/[0.05]" : "hover:bg-muted/40",
        dragHandleProps?.isDragging &&
          "z-10 cursor-grabbing rounded-md shadow-lg ring-1 ring-primary/50",
      )}
    >
      {isLive && (
        <span
          aria-hidden
          className="absolute bottom-2 left-0 top-2 w-[3px] rounded-r-full bg-primary"
        />
      )}
      <div className="flex items-center gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <button
            type="button"
            className={cn(
              "-ml-2 flex-shrink-0 cursor-grab p-1 text-muted-foreground/40 opacity-0 transition-opacity hover:text-muted-foreground group-hover:opacity-100 active:cursor-grabbing",
              dragHandleProps?.isDragging && "cursor-grabbing opacity-100",
            )}
            aria-label={t("provider.dragHandle")}
            {...(dragHandleProps?.attributes ?? {})}
            {...(dragHandleProps?.listeners ?? {})}
          >
            <GripVertical className="h-4 w-4" />
          </button>

          <ProviderIcon
            icon={provider.icon}
            name={provider.name}
            color={provider.iconColor}
            size={22}
          />

          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <h3
                className="min-w-0 truncate text-[14.5px] font-medium leading-snug"
                title={accountEmail || provider.name}
              >
                {accountEmail || provider.name}
              </h3>

              {isLive && (
                <span className="shrink-0 whitespace-nowrap text-[12px] font-medium text-primary">
                  {isAdditiveMode
                    ? t("provider.inConfig", { defaultValue: "In config" })
                    : t("provider.live", { defaultValue: "In use" })}
                </span>
              )}

              {isOmo && (
                <span className="text-[11px] font-medium text-muted-foreground">
                  OMO
                </span>
              )}

              {isOmoSlim && (
                <span className="text-[11px] font-medium text-muted-foreground">
                  Slim
                </span>
              )}

              {isProxyRunning && isInFailoverQueue && health && hasFailures && (
                <ProviderHealthBadge
                  consecutiveFailures={health.consecutive_failures}
                />
              )}
            </div>

            <div className="flex min-w-0 items-center gap-2 text-[12px] text-muted-foreground">
              <span className="min-w-0 truncate">
                {accountEmail ? provider.name : null}
                {!accountEmail && displayUrl ? (
                  <button
                    type="button"
                    onClick={handleOpenWebsite}
                    className={cn(
                      "max-w-[300px] truncate text-left",
                      isClickableUrl
                        ? "hover:text-foreground hover:underline"
                        : "cursor-default",
                    )}
                    title={displayUrl}
                    disabled={!isClickableUrl}
                  >
                    {displayUrl}
                  </button>
                ) : null}
              </span>
              {isAutoFailoverEnabled && isInFailoverQueue && failoverPriority && (
                <FailoverPriorityBadge
                  priority={failoverPriority}
                  className="shrink-0 whitespace-nowrap"
                />
              )}
            </div>
          </div>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-3">
          <div className="relative z-20 ml-auto">
            <div className="flex items-center gap-1">
              {isOfficial ? (
                <SubscriptionQuotaFooter
                  appId={appId}
                  providerId={quotaProviderId}
                  inline={true}
                />
              ) : hasMultiplePlans ? (
                <div className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400">
                  <span className="font-medium">
                    {t("usage.multiplePlans", {
                      count: usage?.data?.length || 0,
                      defaultValue: "{{count}} plans",
                    })}
                  </span>
                </div>
              ) : (
                <UsageFooter
                  provider={provider}
                  providerId={provider.id}
                  appId={appId}
                  usageEnabled={usageEnabled}
                  isCurrent={isCurrent}
                  isInConfig={isInConfig}
                  inline={true}
                />
              )}
              {hasMultiplePlans && (
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setIsExpanded(!isExpanded);
                  }}
                  className="p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors text-gray-500 dark:text-gray-400 flex-shrink-0"
                  title={
                    isExpanded
                      ? t("usage.collapse", { defaultValue: "Collapse" })
                      : t("usage.expand", { defaultValue: "Expand" })
                  }
                >
                  {isExpanded ? (
                    <ChevronUp size={14} />
                  ) : (
                    <ChevronDown size={14} />
                  )}
                </button>
              )}
            </div>
          </div>
          <div className="shrink-0">
            <ProviderActions
              appId={appId}
              isCurrent={isCurrent}
              isInConfig={isInConfig}
              isTesting={isTesting}
              isOmo={isAnyOmo}
              onSwitch={() => onSwitch(provider)}
              onEdit={() => onEdit(provider)}
              onDuplicate={() => onDuplicate(provider)}
              onTest={
                onTest && !isOfficial ? () => onTest(provider) : undefined
              }
              onConfigureUsage={
                isOfficial ? undefined : () => onConfigureUsage(provider)
              }
              onDelete={() => onDelete(provider)}
              onRemoveFromConfig={
                onRemoveFromConfig
                  ? () => onRemoveFromConfig(provider)
                  : undefined
              }
              onDisableOmo={handleDisableAnyOmo}
              onOpenTerminal={
                onOpenTerminal ? () => onOpenTerminal(provider) : undefined
              }
              isAutoFailoverEnabled={isAutoFailoverEnabled}
              isInFailoverQueue={isInFailoverQueue}
              onToggleFailover={onToggleFailover}
              isDefaultModel={isDefaultModel}
              onSetAsDefault={onSetAsDefault}
            />
          </div>
        </div>
      </div>

      {isExpanded && hasMultiplePlans && (
        <div className="mt-4 pt-4 border-t border-border-default">
          <UsageFooter
            provider={provider}
            providerId={provider.id}
            appId={appId}
            usageEnabled={usageEnabled}
            isCurrent={isCurrent}
            isInConfig={isInConfig}
            inline={false}
          />
        </div>
      )}
    </div>
  );
}
