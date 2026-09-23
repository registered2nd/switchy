import React, { useState } from "react";
import { Play, Wand2, Eye, EyeOff, Save } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Provider, UsageScript, UsageData } from "@/types";
import { usageApi, settingsApi, type AppId } from "@/lib/api";
import { copilotGetUsage, copilotGetUsageForAccount } from "@/lib/api/copilot";
import { useSettingsQuery } from "@/lib/query";
import { resolveManagedAccountId } from "@/lib/authBinding";
import { extractCodexBaseUrl } from "@/utils/providerConfigUtils";
import JsonEditor from "./JsonEditor";
import * as prettier from "prettier/standalone";
import * as parserBabel from "prettier/parser-babel";
import * as pluginEstree from "prettier/plugins/estree";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { TEMPLATE_TYPES, PROVIDER_TYPES } from "@/config/constants";

interface UsageScriptModalProps {
  provider: Provider;
  appId: AppId;
  isOpen: boolean;
  onClose: () => void;
  onSave: (script: UsageScript) => void;
}

// Builds the preset templates (localized)
const generatePresetTemplates = (
  t: (key: string) => string,
): Record<string, string> => ({
  [TEMPLATE_TYPES.CUSTOM]: `({
  request: {
    url: "",
    method: "GET",
    headers: {}
  },
  extractor: function(response) {
    return {
      remaining: 0,
      unit: "USD"
    };
  }
})`,

  [TEMPLATE_TYPES.GENERAL]: `({
  request: {
    url: "{{baseUrl}}/user/balance",
    method: "GET",
    headers: {
      "Authorization": "Bearer {{apiKey}}",
      "User-Agent": "switchy/1.0"
    }
  },
  extractor: function(response) {
    return {
      isValid: response.is_active || true,
      remaining: response.balance,
      unit: "USD"
    };
  }
})`,

  [TEMPLATE_TYPES.NEW_API]: `({
  request: {
    url: "{{baseUrl}}/api/user/self",
    method: "GET",
    headers: {
      "Content-Type": "application/json",
      "Authorization": "Bearer {{accessToken}}",
      "New-Api-User": "{{userId}}"
    },
  },
  extractor: function (response) {
    if (response.success && response.data) {
      return {
        planName: response.data.group || "${t("usageScript.defaultPlan")}",
        remaining: response.data.quota / 500000,
        used: response.data.used_quota / 500000,
        total: (response.data.quota + response.data.used_quota) / 500000,
        unit: "USD",
      };
    }
    return {
      isValid: false,
      invalidMessage: response.message || "${t("usageScript.queryFailedMessage")}"
    };
  },
})`,

  // GitHub Copilot needs no script; it uses a dedicated API
  [TEMPLATE_TYPES.GITHUB_COPILOT]: "",

  // Coding Plan needs no script; it uses a dedicated Rust query
  [TEMPLATE_TYPES.TOKEN_PLAN]: "",
});

// i18n keys for template names
const TEMPLATE_NAME_KEYS: Record<string, string> = {
  [TEMPLATE_TYPES.CUSTOM]: "usageScript.templateCustom",
  [TEMPLATE_TYPES.GENERAL]: "usageScript.templateGeneral",
  [TEMPLATE_TYPES.NEW_API]: "usageScript.templateNewAPI",
  [TEMPLATE_TYPES.GITHUB_COPILOT]: "usageScript.templateCopilot",
  [TEMPLATE_TYPES.TOKEN_PLAN]: "usageScript.templateTokenPlan",
};

/** Coding Plan provider options */
const TOKEN_PLAN_PROVIDERS = [
  { id: "kimi", label: "Kimi For Coding", pattern: /api\.kimi\.com\/coding/i },
  {
    id: "zhipu",
    label: "Zhipu GLM",
    pattern: /bigmodel\.cn|api\.z\.ai/i,
  },
  {
    id: "minimax",
    label: "MiniMax",
    pattern: /api\.minimaxi?\.com|api\.minimax\.io/i,
  },
] as const;

/** Detect the Coding Plan provider from the base URL */
function detectTokenPlanProvider(baseUrl: string | undefined): string | null {
  if (!baseUrl) return null;
  for (const cp of TOKEN_PLAN_PROVIDERS) {
    if (cp.pattern.test(baseUrl)) return cp.id;
  }
  return null;
}

const UsageScriptModal: React.FC<UsageScriptModalProps> = ({
  provider,
  appId,
  isOpen,
  onClose,
  onSave,
}) => {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: settingsData } = useSettingsQuery();
  const [showUsageConfirm, setShowUsageConfirm] = useState(false);

  // Localized preset templates
  const PRESET_TEMPLATES = generatePresetTemplates(t);

  // Pull the API key and base URL from the provider's settingsConfig
  const getProviderCredentials = (): {
    apiKey: string | undefined;
    baseUrl: string | undefined;
  } => {
    try {
      const config = provider.settingsConfig;
      if (!config) return { apiKey: undefined, baseUrl: undefined };

      // Handle each app's config format
      if (appId === "claude") {
        // Claude: { env: { ANTHROPIC_AUTH_TOKEN | ANTHROPIC_API_KEY, ANTHROPIC_BASE_URL } }
        const env = (config as any).env || {};
        return {
          apiKey: env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY,
          baseUrl: env.ANTHROPIC_BASE_URL,
        };
      } else if (appId === "codex") {
        // Codex: { auth: { OPENAI_API_KEY }, config: TOML string with base_url }
        const auth = (config as any).auth || {};
        const configToml = (config as any).config || "";
        return {
          apiKey: auth.OPENAI_API_KEY,
          baseUrl: extractCodexBaseUrl(configToml),
        };
      } else if (appId === "gemini") {
        // Gemini: { env: { GEMINI_API_KEY, GOOGLE_GEMINI_BASE_URL } }
        const env = (config as any).env || {};
        return {
          apiKey: env.GEMINI_API_KEY,
          baseUrl: env.GOOGLE_GEMINI_BASE_URL,
        };
      }
      return { apiKey: undefined, baseUrl: undefined };
    } catch (error) {
      console.error("Failed to extract provider credentials:", error);
      return { apiKey: undefined, baseUrl: undefined };
    }
  };

  const providerCredentials = getProviderCredentials();

  const [script, setScript] = useState<UsageScript>(() => {
    const savedScript = provider.meta?.usage_script;
    if (savedScript) {
      // Existing config: coding_plan without codingPlanProvider gets it detected
      if (
        savedScript.templateType === TEMPLATE_TYPES.TOKEN_PLAN &&
        !savedScript.codingPlanProvider
      ) {
        return {
          ...savedScript,
          codingPlanProvider:
            detectTokenPlanProvider(providerCredentials.baseUrl) || "kimi",
        };
      }
      return savedScript;
    }

    // New config: initialize Coding Plan when the URL matches
    const autoDetected = detectTokenPlanProvider(providerCredentials.baseUrl);
    if (autoDetected) {
      return {
        enabled: false,
        language: "javascript" as const,
        code: "",
        timeout: 10,
        codingPlanProvider: autoDetected,
      };
    }

    return {
      enabled: false,
      language: "javascript" as const,
      code: PRESET_TEMPLATES[TEMPLATE_TYPES.GENERAL],
      timeout: 10,
    };
  });

  const [testing, setTesting] = useState(false);

  // Strict validation on blur: valid integer only
  const validateTimeout = (value: string): number => {
    const num = Number(value);
    if (isNaN(num) || value.trim() === "") {
      return 10;
    }
    if (!Number.isInteger(num)) {
      toast.warning(
        t("usageScript.timeoutMustBeInteger") ||
          "Timeout must be an integer, decimal part ignored",
      );
    }
    if (num < 0) {
      toast.error(
        t("usageScript.timeoutCannotBeNegative") ||
          "Timeout cannot be negative",
      );
      return 10;
    }
    return Math.floor(num);
  };

  // Strict validation on blur: auto-query interval
  const validateAndClampInterval = (value: string): number => {
    const num = Number(value);
    if (isNaN(num) || value.trim() === "") {
      return 0;
    }
    if (!Number.isInteger(num)) {
      toast.warning(
        t("usageScript.intervalMustBeInteger") ||
          "Interval must be an integer, decimal part ignored",
      );
    }
    if (num < 0) {
      toast.error(
        t("usageScript.intervalCannotBeNegative") ||
          "Interval cannot be negative",
      );
      return 0;
    }
    const clamped = Math.max(0, Math.min(1440, Math.floor(num)));
    if (clamped !== num && num > 0) {
      toast.info(
        t("usageScript.intervalAdjusted", { value: clamped }) ||
          `Interval adjusted to ${clamped} minutes`,
      );
    }
    return clamped;
  };

  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(
    () => {
      const existingScript = provider.meta?.usage_script;
      // Copilot providers default to the Copilot template
      if (provider.meta?.providerType === PROVIDER_TYPES.GITHUB_COPILOT) {
        return TEMPLATE_TYPES.GITHUB_COPILOT;
      }
      // Prefer the saved templateType
      if (existingScript?.templateType) {
        return existingScript.templateType as string;
      }
      // Backward compatibility: infer the template type from the fields
      // NEW_API template (has accessToken or userId)
      if (existingScript?.accessToken || existingScript?.userId) {
        return TEMPLATE_TYPES.NEW_API;
      }
      // GENERAL template (has apiKey or baseUrl)
      if (existingScript?.apiKey || existingScript?.baseUrl) {
        return TEMPLATE_TYPES.GENERAL;
      }
      // New config: pick the Coding Plan template when the URL matches a Coding Plan provider
      if (detectTokenPlanProvider(providerCredentials.baseUrl)) {
        return TEMPLATE_TYPES.TOKEN_PLAN;
      }
      // Default to GENERAL (matches the default code template)
      return TEMPLATE_TYPES.GENERAL;
    },
  );

  const [showApiKey, setShowApiKey] = useState(false);
  const [showAccessToken, setShowAccessToken] = useState(false);

  const handleEnableToggle = (checked: boolean) => {
    if (checked && !settingsData?.usageConfirmed) {
      setShowUsageConfirm(true);
    } else {
      setScript({ ...script, enabled: checked });
    }
  };

  const handleUsageConfirm = async () => {
    setShowUsageConfirm(false);
    try {
      if (settingsData) {
        const rest = settingsData;
        await settingsApi.save({ ...rest, usageConfirmed: true });
        await queryClient.invalidateQueries({ queryKey: ["settings"] });
      }
    } catch (error) {
      console.error("Failed to save usage confirmed:", error);
    }
    setScript({ ...script, enabled: true });
  };

  const handleSave = () => {
    // Copilot and Coding Plan templates skip script validation
    if (
      selectedTemplate !== TEMPLATE_TYPES.GITHUB_COPILOT &&
      selectedTemplate !== TEMPLATE_TYPES.TOKEN_PLAN
    ) {
      if (script.enabled && !script.code.trim()) {
        toast.error(t("usageScript.scriptEmpty"));
        return;
      }
      if (script.enabled && !script.code.includes("return")) {
        toast.error(t("usageScript.mustHaveReturn"), { duration: 5000 });
        return;
      }
    }
    // Save the selected template type
    const scriptWithTemplate = {
      ...script,
      templateType: selectedTemplate as
        | "custom"
        | "general"
        | "newapi"
        | "github_copilot"
        | "token_plan"
        | undefined,
    };
    onSave(scriptWithTemplate);
    onClose();
  };

  const handleTest = async () => {
    setTesting(true);
    try {
      // Coding Plan uses a dedicated API
      if (selectedTemplate === TEMPLATE_TYPES.TOKEN_PLAN) {
        const config = provider.settingsConfig as Record<string, any>;
        const baseUrl: string = config?.env?.ANTHROPIC_BASE_URL ?? "";
        const apiKey: string =
          config?.env?.ANTHROPIC_AUTH_TOKEN ??
          config?.env?.ANTHROPIC_API_KEY ??
          "";
        const { subscriptionApi } = await import("@/lib/api/subscription");
        const quota = await subscriptionApi.getCodingPlanQuota(baseUrl, apiKey);
        if (quota.success && quota.tiers.length > 0) {
          const summary = quota.tiers
            .map((tier) => `${tier.name}: ${Math.round(tier.utilization)}%`)
            .join(", ");
          toast.success(`${t("usageScript.testSuccess")}${summary}`, {
            duration: 3000,
            closeButton: true,
          });
          // Convert the result to UsageResult and update the cache
          const usageData = quota.tiers.map((tier) => ({
            planName: tier.name,
            remaining: 100 - tier.utilization,
            total: 100,
            used: tier.utilization,
            unit: "%",
          }));
          queryClient.setQueryData(["usage", provider.id, appId], {
            success: true,
            data: usageData,
          });
        } else {
          toast.error(
            `${t("usageScript.testFailed")}: ${quota.error || t("endpointTest.noResult")}`,
            { duration: 5000 },
          );
        }
        return;
      }

      // Copilot uses a dedicated API
      if (selectedTemplate === TEMPLATE_TYPES.GITHUB_COPILOT) {
        const accountId = resolveManagedAccountId(
          provider.meta,
          PROVIDER_TYPES.GITHUB_COPILOT,
        );
        const usage = accountId
          ? await copilotGetUsageForAccount(accountId)
          : await copilotGetUsage();
        const premium = usage.quota_snapshots.premium_interactions;
        const used = premium.entitlement - premium.remaining;
        const summary = `[${usage.copilot_plan}] ${t("usage.remaining")} ${premium.remaining}/${premium.entitlement} (${t("usageScript.resetDate")}: ${usage.quota_reset_date})`;
        toast.success(`${t("usageScript.testSuccess")}${summary}`, {
          duration: 3000,
          closeButton: true,
        });
        // Update the cache
        queryClient.setQueryData(["usage", provider.id, appId], {
          success: true,
          data: [
            {
              planName: usage.copilot_plan,
              remaining: premium.remaining,
              total: premium.entitlement,
              used: used,
              unit: t("usageScript.premiumRequests"),
            },
          ],
        });
        return;
      }

      const result = await usageApi.testScript(
        provider.id,
        appId,
        script.code,
        script.timeout,
        script.apiKey,
        script.baseUrl,
        script.accessToken,
        script.userId,
        selectedTemplate as "custom" | "general" | "newapi" | undefined,
      );
      if (result.success && result.data && result.data.length > 0) {
        const summary = result.data
          .map((plan: UsageData) => {
            const planInfo = plan.planName ? `[${plan.planName}]` : "";
            return `${planInfo} ${t("usage.remaining")} ${plan.remaining} ${plan.unit}`;
          })
          .join(", ");
        toast.success(`${t("usageScript.testSuccess")}${summary}`, {
          duration: 3000,
          closeButton: true,
        });

        // After a successful test, update the main list's usage cache
        queryClient.setQueryData(["usage", provider.id, appId], result);
      } else {
        toast.error(
          `${t("usageScript.testFailed")}: ${result.error || t("endpointTest.noResult")}`,
          {
            duration: 5000,
          },
        );
      }
    } catch (error: any) {
      toast.error(
        `${t("usageScript.testFailed")}: ${error?.message || t("common.unknown")}`,
        {
          duration: 5000,
        },
      );
    } finally {
      setTesting(false);
    }
  };

  const handleFormat = async () => {
    try {
      const formatted = await prettier.format(script.code, {
        parser: "babel",
        plugins: [parserBabel as any, pluginEstree as any],
        semi: true,
        singleQuote: false,
        tabWidth: 2,
        printWidth: 80,
      });
      setScript({ ...script, code: formatted.trim() });
      toast.success(t("usageScript.formatSuccess"), {
        duration: 1000,
        closeButton: true,
      });
    } catch (error: any) {
      toast.error(
        `${t("usageScript.formatFailed")}: ${error?.message || t("jsonEditor.invalidJson")}`,
        {
          duration: 3000,
        },
      );
    }
  };

  const handleUsePreset = (presetName: string) => {
    const preset = PRESET_TEMPLATES[presetName];
    if (preset !== undefined) {
      if (presetName === TEMPLATE_TYPES.CUSTOM) {
        // Custom mode: the script holds the full URL and credentials instead of relying on variable substitution
        // This avoids same-origin check problems
        // To use variables, set baseUrl/apiKey in the config manually
        setScript({
          ...script,
          code: preset,
          // Clear credentials; the user can enter them or leave them empty
          apiKey: undefined,
          baseUrl: undefined,
          accessToken: undefined,
          userId: undefined,
        });
      } else if (presetName === TEMPLATE_TYPES.GENERAL) {
        setScript({
          ...script,
          code: preset,
          accessToken: undefined,
          userId: undefined,
        });
      } else if (presetName === TEMPLATE_TYPES.NEW_API) {
        setScript({
          ...script,
          code: preset,
          apiKey: undefined,
        });
      } else if (presetName === TEMPLATE_TYPES.GITHUB_COPILOT) {
        // Copilot needs no script or credentials; it uses a dedicated API
        setScript({
          ...script,
          code: "",
          apiKey: undefined,
          baseUrl: undefined,
          accessToken: undefined,
          userId: undefined,
        });
      } else if (presetName === TEMPLATE_TYPES.TOKEN_PLAN) {
        // Coding Plan needs no script; it uses a native Rust query
        const autoDetected = detectTokenPlanProvider(
          providerCredentials.baseUrl,
        );
        setScript({
          ...script,
          code: "",
          apiKey: undefined,
          baseUrl: undefined,
          accessToken: undefined,
          userId: undefined,
          codingPlanProvider:
            script.codingPlanProvider || autoDetected || "kimi",
        });
      }
      setSelectedTemplate(presetName);
    }
  };

  const shouldShowCredentialsConfig =
    selectedTemplate === TEMPLATE_TYPES.GENERAL ||
    selectedTemplate === TEMPLATE_TYPES.NEW_API;

  const footer = (
    <>
      <div className="flex gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={handleTest}
          disabled={!script.enabled || testing}
        >
          <Play size={14} className="mr-1" />
          {testing ? t("usageScript.testing") : t("usageScript.testScript")}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={handleFormat}
          disabled={!script.enabled}
          title={t("usageScript.format")}
        >
          <Wand2 size={14} className="mr-1" />
          {t("usageScript.format")}
        </Button>
      </div>

      <div className="flex gap-2">
        <Button
          variant="outline"
          onClick={onClose}
          className="border-border/20 hover:bg-accent hover:text-accent-foreground"
        >
          {t("common.cancel")}
        </Button>
        <Button
          onClick={handleSave}
          className="bg-primary text-primary-foreground hover:bg-primary/90"
        >
          <Save size={16} className="mr-2" />
          {t("usageScript.saveConfig")}
        </Button>
      </div>
    </>
  );

  return (
    <FullScreenPanel
      isOpen={isOpen}
      title={`${t("usageScript.title")} - ${provider.name}`}
      onClose={onClose}
      footer={footer}
    >
      <div className="glass rounded-xl border border-white/10 px-6 py-4 flex items-center justify-between gap-4">
        <p className="text-base font-medium leading-none text-foreground">
          {t("usageScript.enableUsageQuery")}
        </p>
        <Switch
          checked={script.enabled}
          onCheckedChange={handleEnableToggle}
          aria-label={t("usageScript.enableUsageQuery")}
        />
      </div>

      {script.enabled && (
        <div className="space-y-6">
          {/* Preset template picker */}
          <div className="space-y-4 glass rounded-xl border border-white/10 p-6">
            <Label className="text-base font-medium">
              {t("usageScript.presetTemplate")}
            </Label>
            <div className="flex gap-2 flex-wrap">
              {Object.keys(PRESET_TEMPLATES)
                .filter((name) => {
                  const isCopilotProvider =
                    provider.meta?.providerType === "github_copilot";
                  // Copilot providers show only the Copilot template
                  if (isCopilotProvider) {
                    return name === TEMPLATE_TYPES.GITHUB_COPILOT;
                  }
                  // Other providers hide the Copilot template
                  return name !== TEMPLATE_TYPES.GITHUB_COPILOT;
                })
                .map((name) => {
                  const isSelected = selectedTemplate === name;
                  return (
                    <Button
                      key={name}
                      type="button"
                      variant={isSelected ? "default" : "outline"}
                      size="sm"
                      className={cn(
                        "rounded-lg border",
                        isSelected
                          ? "shadow-sm"
                          : "bg-background text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                      )}
                      onClick={() => handleUsePreset(name)}
                    >
                      {t(TEMPLATE_NAME_KEYS[name])}
                    </Button>
                  );
                })}
            </div>

            {/* Custom mode: variables and their values */}
            {selectedTemplate === TEMPLATE_TYPES.CUSTOM && (
              <div className="space-y-2 border-t border-white/10 pt-3">
                <h4 className="text-sm font-medium text-foreground">
                  {t("usageScript.supportedVariables")}
                </h4>
                <div className="space-y-1 text-xs">
                  {/* baseUrl */}
                  <div className="flex items-center gap-2 py-1">
                    <code className="text-emerald-500 dark:text-emerald-400 font-mono shrink-0">
                      {"{{baseUrl}}"}
                    </code>
                    <span className="text-muted-foreground/50">=</span>
                    {providerCredentials.baseUrl ? (
                      <code className="text-foreground/70 break-all font-mono">
                        {providerCredentials.baseUrl}
                      </code>
                    ) : (
                      <span className="text-muted-foreground/50 italic">
                        {t("common.notSet") || "Not Set"}
                      </span>
                    )}
                  </div>

                  {/* apiKey */}
                  <div className="flex items-center gap-2 py-1">
                    <code className="text-emerald-500 dark:text-emerald-400 font-mono shrink-0">
                      {"{{apiKey}}"}
                    </code>
                    <span className="text-muted-foreground/50">=</span>
                    {providerCredentials.apiKey ? (
                      <>
                        {showApiKey ? (
                          <code className="text-foreground/70 break-all font-mono">
                            {providerCredentials.apiKey}
                          </code>
                        ) : (
                          <code className="text-foreground/70 font-mono">
                            ••••••••
                          </code>
                        )}
                        <button
                          type="button"
                          onClick={() => setShowApiKey(!showApiKey)}
                          className="text-muted-foreground hover:text-foreground transition-colors ml-1"
                          aria-label={
                            showApiKey
                              ? t("apiKeyInput.hide")
                              : t("apiKeyInput.show")
                          }
                        >
                          {showApiKey ? (
                            <EyeOff size={12} />
                          ) : (
                            <Eye size={12} />
                          )}
                        </button>
                      </>
                    ) : (
                      <span className="text-muted-foreground/50 italic">
                        {t("common.notSet") || "Not Set"}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            )}

            {/* Copilot mode: automatic auth hint */}
            {selectedTemplate === TEMPLATE_TYPES.GITHUB_COPILOT && (
              <div className="space-y-2 border-t border-white/10 pt-3">
                <p className="text-sm text-muted-foreground">
                  {t("usageScript.copilotAutoAuth")}
                </p>
              </div>
            )}

            {/* Coding Plan mode: provider picker */}
            {selectedTemplate === TEMPLATE_TYPES.TOKEN_PLAN && (
              <div className="space-y-3 border-t border-white/10 pt-3">
                <p className="text-sm text-muted-foreground">
                  {t("usageScript.tokenPlanHint")}
                </p>
                <div className="flex gap-2 flex-wrap">
                  {TOKEN_PLAN_PROVIDERS.map((cp) => (
                    <Button
                      key={cp.id}
                      type="button"
                      variant={
                        script.codingPlanProvider === cp.id
                          ? "default"
                          : "outline"
                      }
                      size="sm"
                      className={cn(
                        "rounded-lg border",
                        script.codingPlanProvider === cp.id
                          ? "shadow-sm"
                          : "bg-background text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                      )}
                      onClick={() =>
                        setScript({
                          ...script,
                          codingPlanProvider: cp.id,
                        })
                      }
                    >
                      {cp.label}
                    </Button>
                  ))}
                </div>
              </div>
            )}

            {/* Credentials */}
            {shouldShowCredentialsConfig && (
              <div className="space-y-4">
                <div className="flex items-start justify-between">
                  <h4 className="text-sm font-medium text-foreground">
                    {t("usageScript.credentialsConfig")}
                  </h4>
                  <p className="text-xs text-muted-foreground">
                    {t("usageScript.credentialsHint")}
                  </p>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  {selectedTemplate === TEMPLATE_TYPES.GENERAL && (
                    <>
                      <div className="space-y-2">
                        <Label htmlFor="usage-api-key">
                          API Key{" "}
                          <span className="text-xs text-muted-foreground font-normal">
                            ({t("usageScript.optional")})
                          </span>
                        </Label>
                        <div className="relative">
                          <Input
                            id="usage-api-key"
                            type={showApiKey ? "text" : "password"}
                            value={script.apiKey || ""}
                            onChange={(e) =>
                              setScript({ ...script, apiKey: e.target.value })
                            }
                            placeholder={t("usageScript.apiKeyPlaceholder")}
                            autoComplete="off"
                            className="border-white/10"
                          />
                          {script.apiKey && (
                            <button
                              type="button"
                              onClick={() => setShowApiKey(!showApiKey)}
                              className="absolute inset-y-0 right-0 flex items-center pr-3 text-muted-foreground hover:text-foreground transition-colors"
                              aria-label={
                                showApiKey
                                  ? t("apiKeyInput.hide")
                                  : t("apiKeyInput.show")
                              }
                            >
                              {showApiKey ? (
                                <EyeOff size={16} />
                              ) : (
                                <Eye size={16} />
                              )}
                            </button>
                          )}
                        </div>
                      </div>

                      <div className="space-y-2">
                        <Label htmlFor="usage-base-url">
                          {t("usageScript.baseUrl")}{" "}
                          <span className="text-xs text-muted-foreground font-normal">
                            ({t("usageScript.optional")})
                          </span>
                        </Label>
                        <Input
                          id="usage-base-url"
                          type="text"
                          value={script.baseUrl || ""}
                          onChange={(e) =>
                            setScript({ ...script, baseUrl: e.target.value })
                          }
                          placeholder={t("usageScript.baseUrlPlaceholder")}
                          autoComplete="off"
                          className="border-white/10"
                        />
                      </div>
                    </>
                  )}

                  {selectedTemplate === TEMPLATE_TYPES.NEW_API && (
                    <>
                      <div className="space-y-2">
                        <Label htmlFor="usage-newapi-base-url">
                          {t("usageScript.baseUrl")}
                        </Label>
                        <Input
                          id="usage-newapi-base-url"
                          type="text"
                          value={script.baseUrl || ""}
                          onChange={(e) =>
                            setScript({ ...script, baseUrl: e.target.value })
                          }
                          placeholder="https://api.newapi.com"
                          autoComplete="off"
                          className="border-white/10"
                        />
                      </div>

                      <div className="space-y-2">
                        <Label htmlFor="usage-access-token">
                          {t("usageScript.accessToken")}
                        </Label>
                        <div className="relative">
                          <Input
                            id="usage-access-token"
                            type={showAccessToken ? "text" : "password"}
                            value={script.accessToken || ""}
                            onChange={(e) =>
                              setScript({
                                ...script,
                                accessToken: e.target.value,
                              })
                            }
                            placeholder={t(
                              "usageScript.accessTokenPlaceholder",
                            )}
                            autoComplete="off"
                            className="border-white/10"
                          />
                          {script.accessToken && (
                            <button
                              type="button"
                              onClick={() =>
                                setShowAccessToken(!showAccessToken)
                              }
                              className="absolute inset-y-0 right-0 flex items-center pr-3 text-muted-foreground hover:text-foreground transition-colors"
                              aria-label={
                                showAccessToken
                                  ? t("apiKeyInput.hide")
                                  : t("apiKeyInput.show")
                              }
                            >
                              {showAccessToken ? (
                                <EyeOff size={16} />
                              ) : (
                                <Eye size={16} />
                              )}
                            </button>
                          )}
                        </div>
                      </div>

                      <div className="space-y-2">
                        <Label htmlFor="usage-user-id">
                          {t("usageScript.userId")}
                        </Label>
                        <Input
                          id="usage-user-id"
                          type="text"
                          value={script.userId || ""}
                          onChange={(e) =>
                            setScript({ ...script, userId: e.target.value })
                          }
                          placeholder={t("usageScript.userIdPlaceholder")}
                          autoComplete="off"
                          className="border-white/10"
                        />
                      </div>
                    </>
                  )}
                </div>
              </div>
            )}

            {/* General settings (always shown) */}
            <div className="grid gap-4 md:grid-cols-2 pt-4 border-t border-white/10">
              {/* Timeout */}
              <div className="space-y-2">
                <Label htmlFor="usage-timeout">
                  {t("usageScript.timeoutSeconds")}
                </Label>
                <Input
                  id="usage-timeout"
                  type="number"
                  min={0}
                  value={script.timeout ?? 10}
                  onChange={(e) =>
                    setScript({
                      ...script,
                      timeout: validateTimeout(e.target.value),
                    })
                  }
                  onBlur={(e) =>
                    setScript({
                      ...script,
                      timeout: validateTimeout(e.target.value),
                    })
                  }
                  className="border-white/10"
                />
              </div>

              {/* Auto-query interval */}
              <div className="space-y-2">
                <Label htmlFor="usage-interval">
                  {t("usageScript.autoIntervalMinutes")}
                </Label>
                <Input
                  id="usage-interval"
                  type="number"
                  min={0}
                  max={1440}
                  value={
                    script.autoQueryInterval ?? script.autoIntervalMinutes ?? 0
                  }
                  onChange={(e) =>
                    setScript({
                      ...script,
                      autoQueryInterval: validateAndClampInterval(
                        e.target.value,
                      ),
                    })
                  }
                  onBlur={(e) =>
                    setScript({
                      ...script,
                      autoQueryInterval: validateAndClampInterval(
                        e.target.value,
                      ),
                    })
                  }
                  className="border-white/10"
                />
              </div>
            </div>
          </div>

          {/* Extractor code (not for Copilot) */}
          {selectedTemplate !== TEMPLATE_TYPES.GITHUB_COPILOT &&
            selectedTemplate !== TEMPLATE_TYPES.TOKEN_PLAN && (
              <div className="space-y-4 glass rounded-xl border border-white/10 p-6">
                <div className="flex items-center justify-between">
                  <Label className="text-base font-medium">
                    {t("usageScript.extractorCode")}
                  </Label>
                  <div className="text-xs text-muted-foreground">
                    {t("usageScript.extractorHint")}
                  </div>
                </div>
                <JsonEditor
                  id="usage-code"
                  value={script.code || ""}
                  onChange={(value) =>
                    setScript((prev) => ({ ...prev, code: value }))
                  }
                  height={480}
                  language="javascript"
                  showMinimap={false}
                />
              </div>
            )}

          {/* Help (not for Copilot) */}
          {selectedTemplate !== TEMPLATE_TYPES.GITHUB_COPILOT &&
            selectedTemplate !== TEMPLATE_TYPES.TOKEN_PLAN && (
              <div className="glass rounded-xl border border-white/10 p-6 text-sm text-foreground/90">
                <h4 className="font-medium mb-2">
                  {t("usageScript.scriptHelp")}
                </h4>
                <div className="space-y-3 text-xs">
                  <div>
                    <strong>{t("usageScript.configFormat")}</strong>
                    <pre className="mt-1 p-2 bg-black/20 text-foreground rounded border border-white/10 text-[10px] overflow-x-auto">
                      {`({
  request: {
    url: "{{baseUrl}}/api/usage",
    method: "POST",
    headers: {
      "Authorization": "Bearer {{apiKey}}",
      "User-Agent": "switchy/1.0"
    }
  },
  extractor: function(response) {
    return {
      isValid: !response.error,
      remaining: response.balance,
      unit: "USD"
    };
  }
})`}
                    </pre>
                  </div>

                  <div>
                    <strong>{t("usageScript.extractorFormat")}</strong>
                    <ul className="mt-1 space-y-0.5 ml-2">
                      <li>{t("usageScript.fieldIsValid")}</li>
                      <li>{t("usageScript.fieldInvalidMessage")}</li>
                      <li>{t("usageScript.fieldRemaining")}</li>
                      <li>{t("usageScript.fieldUnit")}</li>
                      <li>{t("usageScript.fieldPlanName")}</li>
                      <li>{t("usageScript.fieldTotal")}</li>
                      <li>{t("usageScript.fieldUsed")}</li>
                      <li>{t("usageScript.fieldExtra")}</li>
                    </ul>
                  </div>

                  <div className="text-muted-foreground">
                    <strong>{t("usageScript.tips")}</strong>
                    <ul className="mt-1 space-y-0.5 ml-2">
                      <li>
                        {t("usageScript.tip1", {
                          apiKey: "{{apiKey}}",
                          baseUrl: "{{baseUrl}}",
                        })}
                      </li>
                      <li>{t("usageScript.tip2")}</li>
                      <li>{t("usageScript.tip3")}</li>
                    </ul>
                  </div>
                </div>
              </div>
            )}
        </div>
      )}

      <ConfirmDialog
        isOpen={showUsageConfirm}
        variant="info"
        title={t("confirm.usage.title")}
        message={t("confirm.usage.message")}
        confirmText={t("confirm.usage.confirm")}
        onConfirm={() => void handleUsageConfirm()}
        onCancel={() => setShowUsageConfirm(false)}
      />
    </FullScreenPanel>
  );
};

export default UsageScriptModal;
