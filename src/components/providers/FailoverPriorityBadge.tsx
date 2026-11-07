import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";

interface FailoverPriorityBadgeProps {
  priority: number; // 1, 2, 3, ...
  className?: string;
}

/**
 * Switching-order badge
 * Shows the provider's position in the switching order
 */
export function FailoverPriorityBadge({
  priority,
  className,
}: FailoverPriorityBadgeProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        "inline-flex items-center text-[12px] tabular-nums text-muted-foreground",
        className,
      )}
      title={t("failover.priority.tooltip", {
        priority,
        defaultValue: "Position {{priority}} in the switching order",
      })}
    >
      {t("failover.priority.label", {
        priority,
        defaultValue: "{{priority}} in order",
      })}
    </div>
  );
}
