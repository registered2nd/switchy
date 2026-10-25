import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import type { AppId } from "@/lib/api";
import { cn } from "@/lib/utils";
import { PROGRAM_NAMES } from "@/config/companies";

export interface UsedInChip {
  app: AppId;
  inUse: boolean;
  /** Why the chip cannot be picked, when it cannot. */
  unavailable?: string;
}

interface UsedInChipsProps {
  chips: UsedInChip[];
  /** Picks a program; for one that holds several accounts, clicking again takes it out. */
  onPick: (app: AppId, inUse: boolean) => void;
  busy?: AppId | null;
}

/** "Used in: Claude Code · OpenCode": where an account runs, and where it can. */
export function UsedInChips({ chips, onPick, busy }: UsedInChipsProps) {
  const { t } = useTranslation();
  if (chips.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
      <span className="text-muted-foreground">
        {t("account.usedInLabel", { defaultValue: "Used in:" })}
      </span>
      {chips.map((chip) => {
        const removable = chip.inUse && chip.app === "opencode";
        const disabled =
          Boolean(chip.unavailable) ||
          busy === chip.app ||
          (chip.inUse && !removable);
        return (
          <button
            key={chip.app}
            type="button"
            disabled={disabled}
            onClick={() => onPick(chip.app, chip.inUse)}
            title={
              chip.unavailable ??
              (chip.inUse
                ? removable
                  ? t("account.removeFrom", {
                      program: PROGRAM_NAMES[chip.app],
                      defaultValue: "Take it out of {{program}}",
                    })
                  : t("account.inUseIn", {
                      program: PROGRAM_NAMES[chip.app],
                      defaultValue: "In use in {{program}}",
                    })
                : t("account.useIn", {
                    program: PROGRAM_NAMES[chip.app],
                    defaultValue: "Use in {{program}}",
                  }))
            }
            className={cn(
              "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 transition-colors",
              chip.inUse
                ? "border-primary/40 bg-primary/10 font-medium text-primary"
                : "border-border text-muted-foreground hover:border-foreground/40 hover:text-foreground",
              disabled && !chip.inUse && "cursor-not-allowed opacity-50",
              chip.inUse && !removable && "cursor-default",
            )}
          >
            {chip.inUse && <Check className="h-3 w-3" />}
            {PROGRAM_NAMES[chip.app]}
          </button>
        );
      })}
    </div>
  );
}
