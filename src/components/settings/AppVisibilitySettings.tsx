import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { ProviderIcon } from "@/components/ProviderIcon";
import type { SettingsFormState } from "@/hooks/useSettings";
import { COMPANIES } from "@/config/companies";

interface AppVisibilitySettingsProps {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
}

/**
 * Which companies the sidebar lists. A company not shown by default still
 * appears once it holds an account, unless it is turned off here.
 */
export function AppVisibilitySettings({
  settings,
  onChange,
}: AppVisibilitySettingsProps) {
  const { t } = useTranslation();
  const hidden = settings.hiddenCompanies ?? [];

  const toggle = (id: string) => {
    onChange({
      hiddenCompanies: hidden.includes(id)
        ? hidden.filter((h) => h !== id)
        : [...hidden, id],
    });
  };

  return (
    <section className="space-y-2">
      <header className="space-y-1">
        <h3 className="text-sm font-medium">
          {t("settings.appVisibility.title")}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.appVisibility.companiesDescription", {
            defaultValue:
              "Companies in the sidebar. The ones off by default appear once they hold an account.",
          })}
        </p>
      </header>
      <div className="flex flex-wrap gap-1 rounded-md bg-muted p-1">
        {COMPANIES.map((company) => {
          const shown = !hidden.includes(company.id);
          return (
            <Button
              key={company.id}
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => toggle(company.id)}
              className={cn(
                "gap-1.5",
                shown
                  ? "bg-card font-medium text-foreground shadow-[inset_0_-2px_0_hsl(var(--primary))] hover:bg-card"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground",
              )}
              title={
                company.shownByDefault
                  ? undefined
                  : t("settings.appVisibility.whenHolding", {
                      defaultValue: "Shown once it holds an account",
                    })
              }
            >
              <ProviderIcon icon={company.icon} name={company.name} size={14} />
              {company.name}
            </Button>
          );
        })}
      </div>
    </section>
  );
}
