/**
 * Switch-automatically toggle
 *
 * Sits in the main header; turns Switch automatically on or off in one click
 */

import { Loader2 } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import {
  useAutoFailoverEnabled,
  useSetAutoFailoverEnabled,
} from "@/lib/query/failover";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";

interface FailoverToggleProps {
  className?: string;
  activeApp: AppId;
}

export function FailoverToggle({ className, activeApp }: FailoverToggleProps) {
  const { t } = useTranslation();
  const { data: isEnabled = false, isLoading } =
    useAutoFailoverEnabled(activeApp);
  const setEnabled = useSetAutoFailoverEnabled();

  const handleToggle = (checked: boolean) => {
    setEnabled.mutate({ appType: activeApp, enabled: checked });
  };

  const appLabel =
    activeApp === "claude"
      ? "Claude"
      : activeApp === "codex"
        ? "Codex"
        : "Gemini";

  const tooltipText = isEnabled
    ? t("failover.tooltip.enabled", {
        app: appLabel,
        defaultValue: `${appLabel}: Switch automatically is on\nRequests go down the switching order (1, 2, 3…)`,
      })
    : t("failover.tooltip.disabled", {
        app: appLabel,
        defaultValue: `Turn on Switch automatically for ${appLabel}\nMoves to the top of the switching order now, and down it when a request fails`,
      });

  return (
    <label
      className={cn(
        "flex h-8 cursor-pointer items-center gap-2 rounded-md px-2 text-[13px] transition-colors hover:bg-muted/60",
        isEnabled ? "text-foreground" : "text-muted-foreground",
        className,
      )}
      title={tooltipText}
    >
      {(setEnabled.isPending || isLoading) && (
        <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
      )}
      {t("failover.toggleLabel", { defaultValue: "Switch automatically" })}
      <Switch
        checked={isEnabled}
        onCheckedChange={handleToggle}
        disabled={setEnabled.isPending || isLoading}
      />
    </label>
  );
}
