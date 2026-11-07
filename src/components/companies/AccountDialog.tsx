import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { accountsApi, type Account } from "@/lib/api/accounts";
import { accountKeys } from "@/lib/query/accounts";
import { generateUUID } from "@/utils/uuid";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { REGION_NAMES, type Company, type RegionId } from "@/config/companies";

interface AccountDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  company: Company;
  kind: "plan" | "monitor";
  /** Editing this account; adding a new one when absent. */
  account?: Account;
  /** For a Kimi plan key: the Kimi Code sign-in it belongs to. */
  loginProviderId?: string;
  defaultName?: string;
  onSaved?: (account: Account) => void;
}

/** Adds or edits a plan account or a balance monitor: a name, which plan and region, and its key. */
export function AccountDialog({
  open,
  onOpenChange,
  company,
  kind,
  account,
  loginProviderId,
  defaultName,
  onSaved,
}: AccountDialogProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const plans = kind === "plan" ? (company.plans ?? []) : [];
  const [name, setName] = useState("");
  const [planId, setPlanId] = useState<string>("");
  const [region, setRegion] = useState<RegionId>("intl");
  const [secret, setSecret] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(account?.name ?? defaultName ?? "");
    setPlanId(account?.plan ?? plans[0]?.id ?? "");
    setSecret(account?.secret ?? "");
    const firstRegion =
      kind === "plan"
        ? (Object.keys(plans[0]?.regions ?? {})[0] as RegionId | undefined)
        : company.monitorRegions?.[0];
    setRegion((account?.region as RegionId) ?? firstRegion ?? "intl");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, account]);

  const plan = plans.find((p) => p.id === planId);
  const regions: RegionId[] = useMemo(
    () =>
      kind === "plan"
        ? (Object.keys(plan?.regions ?? {}) as RegionId[])
        : (company.monitorRegions ?? []),
    [kind, plan, company.monitorRegions],
  );
  useEffect(() => {
    if (regions.length > 0 && !regions.includes(region)) setRegion(regions[0]);
  }, [regions, region]);

  const consoleUrl =
    kind === "plan"
      ? plan?.regions[region]?.consoleUrl
      : company.monitorConsole?.[region];

  const save = async () => {
    if (!name.trim()) {
      toast.error(
        t("account.nameRequired", { defaultValue: "Give the account a name" }),
      );
      return;
    }
    setSaving(true);
    try {
      const saved: Account = {
        ...(account ?? {
          id: generateUUID(),
          createdAt: Date.now(),
          loginProviderId: loginProviderId ?? null,
        }),
        company: company.id,
        kind,
        plan: kind === "plan" ? planId : null,
        region,
        name: name.trim(),
        secret: secret.trim(),
      } as Account;
      await accountsApi.save(saved);
      await queryClient.invalidateQueries({ queryKey: accountKeys.all });
      // Saving gave the account's programs its new key: show them so.
      await queryClient.invalidateQueries({ queryKey: ["providers"] });
      onSaved?.(saved);
      onOpenChange(false);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  const title =
    kind === "monitor"
      ? account
        ? t("account.editMonitor", { defaultValue: "Edit balance monitor" })
        : t("account.addMonitor", {
            company: company.name,
            defaultValue: "Monitor a {{company}} balance",
          })
      : account
        ? t("account.editAccount", { defaultValue: "Edit account" })
        : t("account.addPlanAccount", {
            company: company.name,
            defaultValue: "Add a {{company}} account",
          });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="account-name">
              {t("account.name", { defaultValue: "Name" })}
            </Label>
            <Input
              id="account-name"
              value={name}
              placeholder={t("account.namePlaceholder", {
                defaultValue: "e.g. the email you subscribed with",
              })}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          {plans.length > 1 && (
            <div className="space-y-1.5">
              <Label>{t("account.plan", { defaultValue: "Plan" })}</Label>
              <Select value={planId} onValueChange={setPlanId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {plans.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          {regions.length > 1 && (
            <div className="space-y-1.5">
              <Label>{t("account.region", { defaultValue: "Service" })}</Label>
              <Select
                value={region}
                onValueChange={(value) => setRegion(value as RegionId)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {regions.map((r) => (
                    <SelectItem key={r} value={r}>
                      {t(`account.regions.${r}`, {
                        defaultValue: REGION_NAMES[r],
                      })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <Label htmlFor="account-secret">
                {t("account.key", { defaultValue: "Key" })}
              </Label>
              {consoleUrl && (
                <button
                  type="button"
                  className="flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
                  onClick={() => void settingsApi.openExternal(consoleUrl)}
                >
                  {t("account.getKey", {
                    company: company.name,
                    defaultValue: "From the {{company}} console",
                  })}
                  <ExternalLink className="h-3 w-3" />
                </button>
              )}
            </div>
            <Input
              id="account-secret"
              type="password"
              autoComplete="off"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
            />
            <p className="text-[12px] text-muted-foreground">
              {kind === "monitor"
                ? t("account.monitorHint", {
                    defaultValue:
                      "Read only to show the balance and what was spent; Switchy runs nothing with it.",
                  })
                : t("account.planHint", {
                    defaultValue:
                      "The key the plan gives you. Choose where it is used on the card.",
                  })}
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={() => void save()} disabled={saving}>
            {account ? t("common.save") : t("common.add")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
