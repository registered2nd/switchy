/**
 * Proxy mode toggle
 *
 * Sits in the main header; turns proxy mode on or off in one click
 * On: takes over the live config. Off: restores the original config
 */

import { Loader2, Radio } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";

interface ProxyToggleProps {
  className?: string;
  activeApp: AppId;
}

export function ProxyToggle({ className, activeApp }: ProxyToggleProps) {
  const { t } = useTranslation();
  const { isRunning, takeoverStatus, setTakeoverForApp, isPending, status } =
    useProxyStatus();

  const handleToggle = async (checked: boolean) => {
    try {
      await setTakeoverForApp({ appType: activeApp, enabled: checked });
    } catch (error) {
      console.error("[ProxyToggle] Toggle takeover failed:", error);
    }
  };

  const takeoverEnabled = takeoverStatus?.[activeApp] || false;

  const appLabel =
    activeApp === "claude"
      ? "Claude"
      : activeApp === "codex"
        ? "Codex"
        : activeApp === "gemini"
          ? "Gemini"
          : "OpenCode";

  const tooltipText = takeoverEnabled
    ? isRunning
      ? t("proxy.takeover.tooltip.active", {
          appLabel,
          address: status?.address,
          port: status?.port,
          defaultValue: `${appLabel} is intercepting - ${status?.address}:${status?.port}\nSwitch provider for hot switching`,
        })
      : t("proxy.takeover.tooltip.broken", {
          appLabel,
          defaultValue: `${appLabel} is intercepting, but proxy service is not running`,
        })
    : t("proxy.takeover.tooltip.inactive", {
        appLabel,
        defaultValue: `Intercept ${appLabel}'s live config to route requests through local proxy`,
      });

  return (
    <label
      className={cn(
        "flex h-8 cursor-pointer items-center gap-2 rounded-md px-2 text-[13px] transition-colors hover:bg-muted/60",
        takeoverEnabled ? "text-foreground" : "text-muted-foreground",
        className,
      )}
      title={tooltipText}
    >
      {isPending ? (
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      ) : (
        <Radio
          className={cn(
            "h-4 w-4 transition-colors",
            takeoverEnabled ? "animate-pulse text-primary" : "text-muted-foreground",
          )}
        />
      )}
      {t("proxy.takeover.label", { defaultValue: "Route through proxy" })}
      <Switch
        checked={takeoverEnabled}
        onCheckedChange={handleToggle}
        disabled={isPending}
      />
    </label>
  );
}
