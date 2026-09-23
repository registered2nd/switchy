import { useTranslation } from "react-i18next";
import { useState, useEffect } from "react";
import {
  ChevronDown,
  ChevronRight,
  FlaskConical,
  Globe,
  Coins,
  Eye,
  EyeOff,
  X,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { ProviderTestConfig, ProviderProxyConfig } from "@/types";

export type PricingModelSourceOption = "inherit" | "request" | "response";

interface ProviderPricingConfig {
  enabled: boolean;
  costMultiplier?: string;
  pricingModelSource: PricingModelSourceOption;
}

interface ProviderAdvancedConfigProps {
  testConfig: ProviderTestConfig;
  proxyConfig: ProviderProxyConfig;
  pricingConfig: ProviderPricingConfig;
  onTestConfigChange: (config: ProviderTestConfig) => void;
  onProxyConfigChange: (config: ProviderProxyConfig) => void;
  onPricingConfigChange: (config: ProviderPricingConfig) => void;
}

/** Build the full URL from ProviderProxyConfig */
function buildProxyUrl(config: ProviderProxyConfig): string {
  if (!config.proxyHost) return "";

  const protocol = config.proxyType || "http";
  const host = config.proxyHost;
  const port = config.proxyPort || (protocol === "socks5" ? 1080 : 7890);

  return `${protocol}://${host}:${port}`;
}

/** Parse a full URL into ProviderProxyConfig */
function parseProxyUrl(url: string): Partial<ProviderProxyConfig> {
  if (!url.trim()) {
    return { proxyHost: undefined, proxyPort: undefined, proxyType: undefined };
  }

  try {
    const parsed = new URL(url);
    const protocol = parsed.protocol.replace(":", "") as
      | "http"
      | "https"
      | "socks5";
    const host = parsed.hostname;
    const port = parsed.port ? parseInt(parsed.port, 10) : undefined;

    return {
      proxyType: protocol,
      proxyHost: host || undefined,
      proxyPort: port,
    };
  } catch {
    // Try a simple parse (not a standard URL)
    const match = url.match(/^(?:(\w+):\/\/)?([^:]+)(?::(\d+))?$/);
    if (match) {
      return {
        proxyType: (match[1] as "http" | "https" | "socks5") || "http",
        proxyHost: match[2] || undefined,
        proxyPort: match[3] ? parseInt(match[3], 10) : undefined,
      };
    }
    return {};
  }
}

export function ProviderAdvancedConfig({
  testConfig,
  proxyConfig,
  pricingConfig,
  onTestConfigChange,
  onProxyConfigChange,
  onPricingConfigChange,
}: ProviderAdvancedConfigProps) {
  const { t } = useTranslation();
  const [isTestConfigOpen, setIsTestConfigOpen] = useState(testConfig.enabled);
  const [isProxyConfigOpen, setIsProxyConfigOpen] = useState(
    proxyConfig.enabled,
  );
  const [isPricingConfigOpen, setIsPricingConfigOpen] = useState(
    pricingConfig.enabled,
  );
  const [showPassword, setShowPassword] = useState(false);

  // Proxy URL input state (built from proxyConfig only on init)
  const [proxyUrl, setProxyUrl] = useState(() => buildProxyUrl(proxyConfig));

  // Whether the user is typing (to tell external updates from user input)
  const [isUserTyping, setIsUserTyping] = useState(false);

  useEffect(() => {
    setIsTestConfigOpen(testConfig.enabled);
  }, [testConfig.enabled]);

  // Sync external proxyConfig.enabled changes into the expanded state
  useEffect(() => {
    setIsProxyConfigOpen(proxyConfig.enabled);
  }, [proxyConfig.enabled]);

  // Sync external pricingConfig.enabled changes into the expanded state
  useEffect(() => {
    setIsPricingConfigOpen(pricingConfig.enabled);
  }, [pricingConfig.enabled]);

  // Sync only when proxyConfig changes externally, not from typing (e.g. form reset, data load)
  useEffect(() => {
    if (!isUserTyping) {
      const newUrl = buildProxyUrl(proxyConfig);
      if (newUrl !== proxyUrl) {
        setProxyUrl(newUrl);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [proxyConfig.proxyType, proxyConfig.proxyHost, proxyConfig.proxyPort]);

  // Handle proxy URL changes (typing does not rebuild the URL)
  const handleProxyUrlChange = (value: string) => {
    setIsUserTyping(true);
    setProxyUrl(value);
    const parsed = parseProxyUrl(value);
    onProxyConfigChange({
      ...proxyConfig,
      ...parsed,
    });
  };

  // End the typing state on blur
  const handleProxyUrlBlur = () => {
    setIsUserTyping(false);
  };

  // Clear the proxy config
  const handleClearProxy = () => {
    setProxyUrl("");
    onProxyConfigChange({
      ...proxyConfig,
      proxyType: undefined,
      proxyHost: undefined,
      proxyPort: undefined,
      proxyUsername: undefined,
      proxyPassword: undefined,
    });
  };

  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-border/50 bg-muted/20">
        <button
          type="button"
          className="flex w-full items-center justify-between p-4 hover:bg-muted/30 transition-colors"
          onClick={() => setIsTestConfigOpen(!isTestConfigOpen)}
        >
          <div className="flex items-center gap-3">
            <FlaskConical className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium">
              {t("providerAdvanced.testConfig", {
                defaultValue: "Model Test Config",
              })}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div
              className="flex items-center gap-2"
              onClick={(e) => e.stopPropagation()}
            >
              <Label
                htmlFor="test-config-enabled"
                className="text-sm text-muted-foreground"
              >
                {t("providerAdvanced.useCustomConfig", {
                  defaultValue: "Use separate config",
                })}
              </Label>
              <Switch
                id="test-config-enabled"
                checked={testConfig.enabled}
                onCheckedChange={(checked) => {
                  onTestConfigChange({ ...testConfig, enabled: checked });
                  if (checked) setIsTestConfigOpen(true);
                }}
              />
            </div>
            {isTestConfigOpen ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </button>
        <div
          className={cn(
            "overflow-hidden transition-all duration-200",
            isTestConfigOpen
              ? "max-h-[500px] opacity-100"
              : "max-h-0 opacity-0",
          )}
        >
          <div className="border-t border-border/50 p-4 space-y-4">
            <p className="text-sm text-muted-foreground">
              {t("providerAdvanced.testConfigDesc", {
                defaultValue:
                  "Configure separate model testing parameters for this provider. Uses global settings when disabled.",
              })}
            </p>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="test-model">
                  {t("providerAdvanced.testModel", {
                    defaultValue: "Test Model",
                  })}
                </Label>
                <Input
                  id="test-model"
                  value={testConfig.testModel || ""}
                  onChange={(e) =>
                    onTestConfigChange({
                      ...testConfig,
                      testModel: e.target.value || undefined,
                    })
                  }
                  placeholder={t("providerAdvanced.testModelPlaceholder", {
                    defaultValue: "Leave empty to use global config",
                  })}
                  disabled={!testConfig.enabled}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="test-timeout">
                  {t("providerAdvanced.timeoutSecs", {
                    defaultValue: "Timeout (seconds)",
                  })}
                </Label>
                <Input
                  id="test-timeout"
                  type="number"
                  min={1}
                  max={300}
                  value={testConfig.timeoutSecs || ""}
                  onChange={(e) =>
                    onTestConfigChange({
                      ...testConfig,
                      timeoutSecs: e.target.value
                        ? parseInt(e.target.value, 10)
                        : undefined,
                    })
                  }
                  placeholder="45"
                  disabled={!testConfig.enabled}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="test-prompt">
                  {t("providerAdvanced.testPrompt", {
                    defaultValue: "Test Prompt",
                  })}
                </Label>
                <Input
                  id="test-prompt"
                  value={testConfig.testPrompt || ""}
                  onChange={(e) =>
                    onTestConfigChange({
                      ...testConfig,
                      testPrompt: e.target.value || undefined,
                    })
                  }
                  placeholder="Who are you?"
                  disabled={!testConfig.enabled}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="degraded-threshold">
                  {t("providerAdvanced.degradedThreshold", {
                    defaultValue: "Degraded Threshold (ms)",
                  })}
                </Label>
                <Input
                  id="degraded-threshold"
                  type="number"
                  min={100}
                  max={60000}
                  value={testConfig.degradedThresholdMs || ""}
                  onChange={(e) =>
                    onTestConfigChange({
                      ...testConfig,
                      degradedThresholdMs: e.target.value
                        ? parseInt(e.target.value, 10)
                        : undefined,
                    })
                  }
                  placeholder="6000"
                  disabled={!testConfig.enabled}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="max-retries">
                  {t("providerAdvanced.maxRetries", {
                    defaultValue: "Max Retries",
                  })}
                </Label>
                <Input
                  id="max-retries"
                  type="number"
                  min={0}
                  max={10}
                  value={testConfig.maxRetries ?? ""}
                  onChange={(e) =>
                    onTestConfigChange({
                      ...testConfig,
                      maxRetries: e.target.value
                        ? parseInt(e.target.value, 10)
                        : undefined,
                    })
                  }
                  placeholder="2"
                  disabled={!testConfig.enabled}
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Proxy config */}
      <div className="rounded-lg border border-border/50 bg-muted/20">
        <button
          type="button"
          className="flex w-full items-center justify-between p-4 hover:bg-muted/30 transition-colors"
          onClick={() => setIsProxyConfigOpen(!isProxyConfigOpen)}
        >
          <div className="flex items-center gap-3">
            <Globe className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium">
              {t("providerAdvanced.proxyConfig", {
                defaultValue: "Proxy Config",
              })}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div
              className="flex items-center gap-2"
              onClick={(e) => e.stopPropagation()}
            >
              <Label
                htmlFor="proxy-config-enabled"
                className="text-sm text-muted-foreground"
              >
                {t("providerAdvanced.useCustomProxy", {
                  defaultValue: "Use separate proxy",
                })}
              </Label>
              <Switch
                id="proxy-config-enabled"
                checked={proxyConfig.enabled}
                onCheckedChange={(checked) => {
                  onProxyConfigChange({ ...proxyConfig, enabled: checked });
                  if (checked) setIsProxyConfigOpen(true);
                }}
              />
            </div>
            {isProxyConfigOpen ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </button>
        <div
          className={cn(
            "overflow-hidden transition-all duration-200",
            isProxyConfigOpen
              ? "max-h-[500px] opacity-100"
              : "max-h-0 opacity-0",
          )}
        >
          <div className="border-t border-border/50 p-4 space-y-3">
            <p className="text-sm text-muted-foreground">
              {t("providerAdvanced.proxyConfigDesc", {
                defaultValue:
                  "Configure separate network proxy for this provider. Uses system proxy or global settings when disabled.",
              })}
            </p>

            {/* Proxy address input (styled like the global proxy) */}
            <div className="flex gap-2">
              <Input
                placeholder="http://127.0.0.1:7890 / socks5://127.0.0.1:1080"
                value={proxyUrl}
                onChange={(e) => handleProxyUrlChange(e.target.value)}
                onBlur={handleProxyUrlBlur}
                className="font-mono text-sm flex-1"
                disabled={!proxyConfig.enabled}
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                disabled={!proxyConfig.enabled || !proxyUrl}
                onClick={handleClearProxy}
                title={t("common.clear", { defaultValue: "Clear" })}
              >
                <X className="h-4 w-4" />
              </Button>
            </div>

            {/* Credentials: username + password (optional) */}
            <div className="flex gap-2">
              <Input
                placeholder={t("providerAdvanced.proxyUsername", {
                  defaultValue: "Username (optional)",
                })}
                value={proxyConfig.proxyUsername || ""}
                onChange={(e) =>
                  onProxyConfigChange({
                    ...proxyConfig,
                    proxyUsername: e.target.value || undefined,
                  })
                }
                className="font-mono text-sm flex-1"
                disabled={!proxyConfig.enabled}
              />
              <div className="relative flex-1">
                <Input
                  type={showPassword ? "text" : "password"}
                  placeholder={t("providerAdvanced.proxyPassword", {
                    defaultValue: "Password (optional)",
                  })}
                  value={proxyConfig.proxyPassword || ""}
                  onChange={(e) =>
                    onProxyConfigChange({
                      ...proxyConfig,
                      proxyPassword: e.target.value || undefined,
                    })
                  }
                  className="font-mono text-sm pr-10"
                  disabled={!proxyConfig.enabled}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
                  onClick={() => setShowPassword(!showPassword)}
                  tabIndex={-1}
                  disabled={!proxyConfig.enabled}
                >
                  {showPassword ? (
                    <EyeOff className="h-4 w-4 text-muted-foreground" />
                  ) : (
                    <Eye className="h-4 w-4 text-muted-foreground" />
                  )}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Pricing config */}
      <div className="rounded-lg border border-border/50 bg-muted/20">
        <button
          type="button"
          className="flex w-full items-center justify-between p-4 hover:bg-muted/30 transition-colors"
          onClick={() => setIsPricingConfigOpen(!isPricingConfigOpen)}
        >
          <div className="flex items-center gap-3">
            <Coins className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium">
              {t("providerAdvanced.pricingConfig", {
                defaultValue: "Pricing Config",
              })}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <div
              className="flex items-center gap-2"
              onClick={(e) => e.stopPropagation()}
            >
              <Label
                htmlFor="pricing-config-enabled"
                className="text-sm text-muted-foreground"
              >
                {t("providerAdvanced.useCustomPricing", {
                  defaultValue: "Use separate config",
                })}
              </Label>
              <Switch
                id="pricing-config-enabled"
                checked={pricingConfig.enabled}
                onCheckedChange={(checked) => {
                  onPricingConfigChange({ ...pricingConfig, enabled: checked });
                  if (checked) setIsPricingConfigOpen(true);
                }}
              />
            </div>
            {isPricingConfigOpen ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            )}
          </div>
        </button>
        <div
          className={cn(
            "overflow-hidden transition-all duration-200",
            isPricingConfigOpen
              ? "max-h-[500px] opacity-100"
              : "max-h-0 opacity-0",
          )}
        >
          <div className="border-t border-border/50 p-4 space-y-4">
            <p className="text-sm text-muted-foreground">
              {t("providerAdvanced.pricingConfigDesc", {
                defaultValue:
                  "Configure separate pricing parameters for this provider. Uses global defaults when disabled.",
              })}
            </p>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="cost-multiplier">
                  {t("providerAdvanced.costMultiplier", {
                    defaultValue: "Cost Multiplier",
                  })}
                </Label>
                <Input
                  id="cost-multiplier"
                  type="number"
                  step="0.01"
                  inputMode="decimal"
                  value={pricingConfig.costMultiplier || ""}
                  onChange={(e) =>
                    onPricingConfigChange({
                      ...pricingConfig,
                      costMultiplier: e.target.value || undefined,
                    })
                  }
                  placeholder={t("providerAdvanced.costMultiplierPlaceholder", {
                    defaultValue: "Leave empty to use global default (1)",
                  })}
                  disabled={!pricingConfig.enabled}
                />
                <p className="text-xs text-muted-foreground">
                  {t("providerAdvanced.costMultiplierHint", {
                    defaultValue:
                      "Actual cost = Base cost × Multiplier, supports decimals like 1.5",
                  })}
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="pricing-model-source">
                  {t("providerAdvanced.pricingModelSourceLabel", {
                    defaultValue: "Pricing Mode",
                  })}
                </Label>
                <Select
                  value={pricingConfig.pricingModelSource}
                  onValueChange={(value) =>
                    onPricingConfigChange({
                      ...pricingConfig,
                      pricingModelSource: value as PricingModelSourceOption,
                    })
                  }
                  disabled={!pricingConfig.enabled}
                >
                  <SelectTrigger id="pricing-model-source">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="inherit">
                      {t("providerAdvanced.pricingModelSourceInherit", {
                        defaultValue: "Inherit global default",
                      })}
                    </SelectItem>
                    <SelectItem value="request">
                      {t("providerAdvanced.pricingModelSourceRequest", {
                        defaultValue: "Request model",
                      })}
                    </SelectItem>
                    <SelectItem value="response">
                      {t("providerAdvanced.pricingModelSourceResponse", {
                        defaultValue: "Response model",
                      })}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("providerAdvanced.pricingModelSourceHint", {
                    defaultValue:
                      "Choose whether to match pricing by request model or response model",
                  })}
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
