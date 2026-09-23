import { Power, Route, Shuffle } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { Switch } from "@/components/ui/switch";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  useProxyTakeoverStatus,
  useSetProxyTakeoverForApp,
} from "@/lib/query/proxy";
import {
  useAutoFailoverEnabled,
  useSetAutoFailoverEnabled,
} from "@/lib/query/failover";
import { AccountPoolPanel } from "@/components/proxy/AccountPoolPanel";

const APP_LABELS = { claude: "Claude", codex: "Codex", gemini: "Gemini" };

interface PoolControlsProps {
  onToggleProxy: (checked: boolean) => Promise<void>;
  isProxyPending: boolean;
}

/**
 * The switches the account pool runs on, in the order they build on each
 * other: the proxy, which apps go through it, failover per app, then
 * rotation and keep-warm.
 */
export function PoolControls({
  onToggleProxy,
  isProxyPending,
}: PoolControlsProps) {
  const { t } = useTranslation();
  const { isRunning } = useProxyStatus();
  const { data: takeoverStatus } = useProxyTakeoverStatus();
  const setTakeoverForApp = useSetProxyTakeoverForApp();

  const handleTakeoverChange = async (appType: string, enabled: boolean) => {
    try {
      await setTakeoverForApp.mutateAsync({ appType, enabled });
      toast.success(
        enabled
          ? t("proxy.takeover.enabled", { app: appType })
          : t("proxy.takeover.disabled", { app: appType }),
        { closeButton: true },
      );
    } catch (error) {
      toast.error(t("proxy.takeover.failed"), {
        description: extractErrorMessage(error) || undefined,
        duration: 12000,
        closeButton: true,
      });
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <Power className="h-4 w-4 text-green-500" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("pool.proxy")}</p>
            <p className="text-xs text-muted-foreground">
              {isRunning
                ? t("settings.advanced.proxy.running")
                : t("settings.advanced.proxy.stopped")}
              {" · "}
              {t("pool.proxyDescription")}
            </p>
          </div>
        </div>
        <Switch
          checked={isRunning}
          onCheckedChange={onToggleProxy}
          disabled={isProxyPending}
        />
      </div>

      <div className="space-y-3">
        <div className="flex items-center gap-3">
          <Route className="h-4 w-4 text-primary" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("pool.route")}</p>
            <p className="text-xs text-muted-foreground">
              {t("pool.routeDescription")}
            </p>
          </div>
        </div>
        <div className="grid gap-2 sm:grid-cols-3 pl-7">
          {(["claude", "codex", "gemini"] as const).map((appType) => (
            <AppSwitch key={appType} label={APP_LABELS[appType]}>
              <Switch
                checked={takeoverStatus?.[appType] ?? false}
                onCheckedChange={(checked) =>
                  void handleTakeoverChange(appType, checked)
                }
                disabled={!isRunning || setTakeoverForApp.isPending}
              />
            </AppSwitch>
          ))}
        </div>
      </div>

      <div className="space-y-3">
        <div className="flex items-center gap-3">
          <Shuffle className="h-4 w-4 text-orange-500" />
          <div className="space-y-0.5">
            <p className="text-sm font-medium">{t("pool.failover")}</p>
            <p className="text-xs text-muted-foreground">
              {t("pool.failoverDescription")}
            </p>
          </div>
        </div>
        <div className="grid gap-2 sm:grid-cols-3 pl-7">
          {(["claude", "codex", "gemini"] as const).map((appType) => (
            <FailoverSwitch
              key={appType}
              appType={appType}
              disabled={!isRunning}
            />
          ))}
        </div>
      </div>

      <div className="border-t border-border/50 pt-6">
        <AccountPoolPanel />
      </div>
    </div>
  );
}

function AppSwitch({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between rounded-md border border-border bg-background/60 px-3 py-2">
      <span className="text-sm font-medium">{label}</span>
      {children}
    </div>
  );
}

function FailoverSwitch({
  appType,
  disabled,
}: {
  appType: keyof typeof APP_LABELS;
  disabled: boolean;
}) {
  const { data: isEnabled = false, isLoading } =
    useAutoFailoverEnabled(appType);
  const setEnabled = useSetAutoFailoverEnabled();
  return (
    <AppSwitch label={APP_LABELS[appType]}>
      <Switch
        checked={isEnabled}
        onCheckedChange={(checked) =>
          setEnabled.mutate({ appType, enabled: checked })
        }
        disabled={disabled || isLoading || setEnabled.isPending}
      />
    </AppSwitch>
  );
}
