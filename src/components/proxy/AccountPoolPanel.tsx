import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { settingsApi, type AccountPoolConfig } from "@/lib/api/settings";

interface AccountPoolPanelProps {
  disabled?: boolean;
}

export function AccountPoolPanel({ disabled = false }: AccountPoolPanelProps) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AccountPoolConfig>({
    enabled: false,
    thresholdPercent: 98,
    blockedExitCountries: ["CN"],
  });
  const [thresholdText, setThresholdText] = useState("98");
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    settingsApi
      .getAccountPoolConfig()
      .then((loaded) => {
        setConfig(loaded);
        setThresholdText(String(loaded.thresholdPercent));
      })
      .catch((e) => console.error("Failed to load account pool config:", e))
      .finally(() => setIsLoading(false));
  }, []);

  const save = async (updates: Partial<AccountPoolConfig>) => {
    const next = { ...config, ...updates };
    setConfig(next);
    try {
      await settingsApi.setAccountPoolConfig(next);
    } catch (e) {
      console.error("Failed to save account pool config:", e);
      toast.error(String(e));
      setConfig(config);
      setThresholdText(String(config.thresholdPercent));
    }
  };

  const commitThreshold = () => {
    const parsed = Math.round(Number(thresholdText));
    if (!Number.isFinite(parsed) || parsed < 50 || parsed > 100) {
      setThresholdText(String(config.thresholdPercent));
      return;
    }
    setThresholdText(String(parsed));
    if (parsed !== config.thresholdPercent) {
      void save({ thresholdPercent: parsed });
    }
  };

  if (isLoading) return null;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5">
          <Label>{t("proxy.accountPool.enabled")}</Label>
          <p className="text-xs text-muted-foreground">
            {t("proxy.accountPool.enabledDescription")}
          </p>
        </div>
        <Switch
          checked={config.enabled}
          disabled={disabled}
          onCheckedChange={(checked) => void save({ enabled: checked })}
        />
      </div>

      <div className="flex items-center justify-between gap-4 pl-4">
        <div className="space-y-0.5">
          <Label htmlFor="account-pool-threshold">
            {t("proxy.accountPool.threshold")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t("proxy.accountPool.thresholdDescription")}
          </p>
        </div>
        <Input
          id="account-pool-threshold"
          type="number"
          min={50}
          max={100}
          className="w-24"
          value={thresholdText}
          disabled={disabled || !config.enabled}
          onChange={(e) => setThresholdText(e.target.value)}
          onBlur={commitThreshold}
        />
      </div>
    </div>
  );
}
