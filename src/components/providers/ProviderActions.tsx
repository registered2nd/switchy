import { useState } from "react";
import {
  BarChart3,
  Check,
  Copy,
  Edit,
  ListMinus,
  ListPlus,
  Loader2,
  Minus,
  MoreHorizontal,
  Plus,
  Terminal,
  TestTube2,
  Trash2,
  Zap,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { AppId } from "@/lib/api";

interface ProviderActionsProps {
  appId?: AppId;
  isCurrent: boolean;
  isInConfig?: boolean;
  isTesting?: boolean;
  isOmo?: boolean;
  onSwitch: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onTest?: () => void;
  onConfigureUsage?: () => void;
  onDelete: () => void;
  onRemoveFromConfig?: () => void;
  onDisableOmo?: () => void;
  onOpenTerminal?: () => void;
  isAutoFailoverEnabled?: boolean;
  isInFailoverQueue?: boolean;
  onToggleFailover?: (enabled: boolean) => void;
  // OpenClaw: default model
  isDefaultModel?: boolean;
  onSetAsDefault?: () => void;
}

export function ProviderActions({
  appId,
  isCurrent,
  isInConfig = false,
  isTesting,
  isOmo = false,
  onSwitch,
  onEdit,
  onDuplicate,
  onTest,
  onConfigureUsage,
  onDelete,
  onRemoveFromConfig,
  onDisableOmo,
  onOpenTerminal,
  isAutoFailoverEnabled = false,
  isInFailoverQueue = false,
  onToggleFailover,
  // OpenClaw: default model
  isDefaultModel = false,
  onSetAsDefault,
}: ProviderActionsProps) {
  const { t } = useTranslation();
  const [menuOpen, setMenuOpen] = useState(false);

  // Additive-mode apps (OpenCode without OMO, and OpenClaw)
  const isAdditiveMode =
    (appId === "opencode" && !isOmo) || appId === "openclaw";

  // Button logic in failover mode (additive-mode and OMO apps do not support failover)
  const isFailoverMode =
    !isAdditiveMode && !isOmo && isAutoFailoverEnabled && onToggleFailover;

  const handleMainButtonClick = () => {
    if (isOmo) {
      if (isCurrent) {
        onDisableOmo?.();
      } else {
        onSwitch();
      }
    } else if (isAdditiveMode) {
      // Additive mode: toggle config membership (add/remove)
      if (isInConfig) {
        if (onRemoveFromConfig) {
          onRemoveFromConfig();
        } else {
          onDelete();
        }
      } else {
        onSwitch(); // add to config
      }
    } else {
      onSwitch();
    }
  };

  const getMainButtonState = () => {
    if (isOmo) {
      if (isCurrent) {
        return {
          disabled: false,
          variant: "secondary" as const,
          className:
            "bg-gray-200 text-muted-foreground hover:bg-gray-200 hover:text-muted-foreground dark:bg-gray-700 dark:hover:bg-gray-700",
          icon: <Check className="h-4 w-4" />,
          text: t("provider.inUse"),
        };
      }
      return {
        disabled: false,
        variant: "default" as const,
        className: "",
        icon: null,
        text: t("provider.enable"),
      };
    }

    // Additive mode (OpenCode without OMO / OpenClaw)
    if (isAdditiveMode) {
      if (isInConfig) {
        return {
          disabled: isDefaultModel === true,
          variant: "secondary" as const,
          className: cn(
            "bg-orange-100 text-orange-600 hover:bg-orange-200 dark:bg-orange-900/50 dark:text-orange-400 dark:hover:bg-orange-900/70",
            isDefaultModel && "opacity-40 cursor-not-allowed",
          ),
          icon: <Minus className="h-4 w-4" />,
          text: t("provider.removeFromConfig", { defaultValue: "Remove" }),
        };
      }
      return {
        disabled: false,
        variant: "default" as const,
        className: "",
        icon: <Plus className="h-4 w-4" />,
        text: t("provider.addToConfig", { defaultValue: "Add" }),
      };
    }

    if (isCurrent) {
      return {
        disabled: true,
        variant: "secondary" as const,
        className:
          "bg-gray-200 text-muted-foreground hover:bg-gray-200 hover:text-muted-foreground dark:bg-gray-700 dark:hover:bg-gray-700",
        icon: <Check className="h-4 w-4" />,
        text: t("provider.inUse"),
      };
    }

    return {
      disabled: false,
      variant: "default" as const,
      className: "",
      icon: null,
      text: t("provider.enable"),
    };
  };

  const buttonState = getMainButtonState();

  const canDelete = isOmo || isAdditiveMode ? true : !isCurrent;
  // The line already says "In use"; a disabled button repeating it adds nothing.
  const showMainButton = isOmo || isAdditiveMode || !isCurrent;

  return (
    <div
      className={cn(
        "flex min-w-[6.75rem] items-center justify-end gap-1 transition-opacity",
        menuOpen
          ? "opacity-100"
          : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
      )}
    >
      {appId === "openclaw" && isInConfig && onSetAsDefault && (
        <Button
          size="sm"
          variant={isDefaultModel ? "secondary" : "default"}
          onClick={isDefaultModel ? undefined : onSetAsDefault}
          disabled={isDefaultModel}
          className={cn(
            "w-fit px-2.5",
            isDefaultModel &&
              "bg-gray-200 text-muted-foreground dark:bg-gray-700 opacity-60 cursor-not-allowed",
          )}
        >
          <Zap className="h-4 w-4" />
          {isDefaultModel
            ? t("provider.isDefault", { defaultValue: "Current Default" })
            : t("provider.setAsDefault", { defaultValue: "Set Default" })}
        </Button>
      )}

      {showMainButton && (
        <Button
          size="sm"
          variant={buttonState.variant}
          onClick={handleMainButtonClick}
          disabled={buttonState.disabled}
          className={cn("min-w-[4.25rem]", buttonState.className)}
        >
          {buttonState.icon}
          {buttonState.text}
        </Button>
      )}

      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger asChild>
          <Button
            size="icon"
            variant="ghost"
            className="h-8 w-8"
            title={t("provider.moreActions", { defaultValue: "More actions" })}
            aria-label={t("provider.moreActions", {
              defaultValue: "More actions",
            })}
          >
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-[13rem]">
          {isFailoverMode && (
            <DropdownMenuItem
              onSelect={() => onToggleFailover(!isInFailoverQueue)}
            >
              {isInFailoverQueue ? (
                <ListMinus className="h-4 w-4" />
              ) : (
                <ListPlus className="h-4 w-4" />
              )}
              {isInFailoverQueue
                ? t("failover.removeQueue", {
                    defaultValue: "Remove from the switching order",
                  })
                : t("failover.addQueue", {
                    defaultValue: "Add to the switching order",
                  })}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={onEdit}>
            <Edit className="h-4 w-4" />
            {t("common.edit")}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onDuplicate}>
            <Copy className="h-4 w-4" />
            {t("provider.duplicate")}
          </DropdownMenuItem>
          {onTest && (
            <DropdownMenuItem onSelect={onTest} disabled={isTesting}>
              {isTesting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <TestTube2 className="h-4 w-4" />
              )}
              {t("modelTest.testProvider", "Test model")}
            </DropdownMenuItem>
          )}
          {onConfigureUsage && (
            <DropdownMenuItem onSelect={onConfigureUsage}>
              <BarChart3 className="h-4 w-4" />
              {t("provider.configureUsage")}
            </DropdownMenuItem>
          )}
          {onOpenTerminal && (
            <DropdownMenuItem onSelect={onOpenTerminal}>
              <Terminal className="h-4 w-4" />
              {t("provider.openTerminal", "Open Terminal")}
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onSelect={onDelete}
            disabled={!canDelete}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 className="h-4 w-4" />
            {t("common.delete")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
