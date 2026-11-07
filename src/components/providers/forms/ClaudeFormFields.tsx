import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { toast } from "sonner";
import { FormLabel } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ChevronDown, ChevronRight, Download, Loader2 } from "lucide-react";
import EndpointSpeedTest from "./EndpointSpeedTest";
import { ApiKeySection, EndpointField, ModelInputWithFetch } from "./shared";
import { CopilotAuthSection } from "./CopilotAuthSection";
import {
  copilotGetModels,
  copilotGetModelsForAccount,
} from "@/lib/api/copilot";
import type { CopilotModel } from "@/lib/api/copilot";
import {
  fetchModelsForConfig,
  showFetchModelsError,
  type FetchedModel,
} from "@/lib/api/model-fetch";
import type {
  ProviderCategory,
  ClaudeApiFormat,
  ClaudeApiKeyField,
} from "@/types";
import type { TemplateValueConfig } from "@/config/claudeProviderPresets";

interface EndpointCandidate {
  url: string;
}

interface ClaudeFormFieldsProps {
  providerId?: string;
  // API Key
  shouldShowApiKey: boolean;
  apiKey: string;
  onApiKeyChange: (key: string) => void;
  category?: ProviderCategory;
  shouldShowApiKeyLink: boolean;
  websiteUrl: string;

  // GitHub Copilot OAuth
  isCopilotPreset?: boolean;
  usesOAuth?: boolean;
  isCopilotAuthenticated?: boolean;
  /** Selected GitHub account ID (multi-account support) */
  selectedGitHubAccountId?: string | null;
  /** GitHub account selection callback (multi-account support) */
  onGitHubAccountSelect?: (accountId: string | null) => void;

  // Template Values
  templateValueEntries: Array<[string, TemplateValueConfig]>;
  templateValues: Record<string, TemplateValueConfig>;
  templatePresetName: string;
  onTemplateValueChange: (key: string, value: string) => void;

  // Base URL
  shouldShowSpeedTest: boolean;
  baseUrl: string;
  onBaseUrlChange: (url: string) => void;
  isEndpointModalOpen: boolean;
  onEndpointModalToggle: (open: boolean) => void;
  onCustomEndpointsChange?: (endpoints: string[]) => void;
  autoSelect: boolean;
  onAutoSelectChange: (checked: boolean) => void;

  // Model Selector
  shouldShowModelSelector: boolean;
  claudeModel: string;
  reasoningModel: string;
  defaultHaikuModel: string;
  defaultSonnetModel: string;
  defaultOpusModel: string;
  onModelChange: (
    field:
      | "ANTHROPIC_MODEL"
      | "ANTHROPIC_REASONING_MODEL"
      | "ANTHROPIC_DEFAULT_HAIKU_MODEL"
      | "ANTHROPIC_DEFAULT_SONNET_MODEL"
      | "ANTHROPIC_DEFAULT_OPUS_MODEL",
    value: string,
  ) => void;

  // Speed Test Endpoints
  speedTestEndpoints: EndpointCandidate[];

  // API Format (for third-party providers that use OpenAI Chat Completions format)
  apiFormat: ClaudeApiFormat;
  onApiFormatChange: (format: ClaudeApiFormat) => void;

  // Auth Field (ANTHROPIC_AUTH_TOKEN or ANTHROPIC_API_KEY)
  apiKeyField: ClaudeApiKeyField;
  onApiKeyFieldChange: (field: ClaudeApiKeyField) => void;

  // Full URL mode
  isFullUrl: boolean;
  onFullUrlChange: (value: boolean) => void;
}

export function ClaudeFormFields({
  providerId,
  shouldShowApiKey,
  apiKey,
  onApiKeyChange,
  category,
  shouldShowApiKeyLink,
  websiteUrl,
  isCopilotPreset,
  usesOAuth,
  isCopilotAuthenticated,
  selectedGitHubAccountId,
  onGitHubAccountSelect,
  templateValueEntries,
  templateValues,
  templatePresetName,
  onTemplateValueChange,
  shouldShowSpeedTest,
  baseUrl,
  onBaseUrlChange,
  isEndpointModalOpen,
  onEndpointModalToggle,
  onCustomEndpointsChange,
  autoSelect,
  onAutoSelectChange,
  shouldShowModelSelector,
  claudeModel,
  reasoningModel,
  defaultHaikuModel,
  defaultSonnetModel,
  defaultOpusModel,
  onModelChange,
  speedTestEndpoints,
  apiFormat,
  onApiFormatChange,
  apiKeyField,
  onApiKeyFieldChange,
  isFullUrl,
  onFullUrlChange,
}: ClaudeFormFieldsProps) {
  const { t } = useTranslation();
  const hasAnyAdvancedValue = !!(
    claudeModel ||
    reasoningModel ||
    defaultHaikuModel ||
    defaultSonnetModel ||
    defaultOpusModel ||
    apiFormat !== "anthropic" ||
    apiKeyField !== "ANTHROPIC_AUTH_TOKEN"
  );
  const [advancedExpanded, setAdvancedExpanded] = useState(hasAnyAdvancedValue);

  // Auto-expand once a preset fills advanced values (only collapsed → expanded, never auto-collapses)
  useEffect(() => {
    if (hasAnyAdvancedValue) {
      setAdvancedExpanded(true);
    }
  }, [hasAnyAdvancedValue]);

  // Models available on Copilot
  const [copilotModels, setCopilotModels] = useState<CopilotModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);

  // Generic model fetching (non-Copilot providers)
  const [fetchedModels, setFetchedModels] = useState<FetchedModel[]>([]);
  const [isFetchingModels, setIsFetchingModels] = useState(false);

  const handleFetchModels = useCallback(() => {
    if (!baseUrl || !apiKey) {
      showFetchModelsError(null, t, {
        hasApiKey: !!apiKey,
        hasBaseUrl: !!baseUrl,
      });
      return;
    }
    setIsFetchingModels(true);
    fetchModelsForConfig(baseUrl, apiKey, isFullUrl)
      .then((models) => {
        setFetchedModels(models);
        if (models.length === 0) {
          toast.info(t("providerForm.fetchModelsEmpty"));
        } else {
          toast.success(
            t("providerForm.fetchModelsSuccess", { count: models.length }),
          );
        }
      })
      .catch((err) => {
        console.warn("[ModelFetch] Failed:", err);
        showFetchModelsError(err, t);
      })
      .finally(() => setIsFetchingModels(false));
  }, [baseUrl, apiKey, isFullUrl, t]);

  // Load available models when the preset is Copilot and authenticated
  useEffect(() => {
    // Not Copilot, or not authenticated: clear the model list
    if (!isCopilotPreset || !isCopilotAuthenticated) {
      setCopilotModels([]);
      setModelsLoading(false);
      return;
    }

    let cancelled = false;
    setModelsLoading(true);
    const fetchModels = selectedGitHubAccountId
      ? copilotGetModelsForAccount(selectedGitHubAccountId)
      : copilotGetModels();

    fetchModels
      .then((models) => {
        if (!cancelled) setCopilotModels(models);
      })
      .catch((err) => {
        console.warn("[Copilot] Failed to fetch models:", err);
        if (!cancelled) {
          toast.error(
            t("copilot.loadModelsFailed", {
              defaultValue: "Failed to load Copilot models",
            }),
          );
        }
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isCopilotPreset, isCopilotAuthenticated, selectedGitHubAccountId]);

  // Model input: free text plus a dropdown
  const renderModelInput = (
    id: string,
    value: string,
    field: ClaudeFormFieldsProps["onModelChange"] extends (
      f: infer F,
      v: string,
    ) => void
      ? F
      : never,
    placeholder?: string,
  ) => {
    if (isCopilotPreset && copilotModels.length > 0) {
      // Group by vendor
      const grouped: Record<string, CopilotModel[]> = {};
      for (const model of copilotModels) {
        const vendor = model.vendor || "Other";
        if (!grouped[vendor]) grouped[vendor] = [];
        grouped[vendor].push(model);
      }
      const vendors = Object.keys(grouped).sort();

      return (
        <div className="flex gap-1">
          <Input
            id={id}
            type="text"
            value={value}
            onChange={(e) => onModelChange(field, e.target.value)}
            placeholder={placeholder}
            autoComplete="off"
            className="flex-1"
          />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="icon" className="shrink-0">
                <ChevronDown className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              className="max-h-64 overflow-y-auto z-[200]"
            >
              {vendors.map((vendor, vi) => (
                <div key={vendor}>
                  {vi > 0 && <DropdownMenuSeparator />}
                  <DropdownMenuLabel>{vendor}</DropdownMenuLabel>
                  {grouped[vendor].map((model) => (
                    <DropdownMenuItem
                      key={model.id}
                      onSelect={() => onModelChange(field, model.id)}
                    >
                      {model.id}
                    </DropdownMenuItem>
                  ))}
                </div>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      );
    }

    if (isCopilotPreset && modelsLoading) {
      return (
        <div className="flex gap-1">
          <Input
            id={id}
            type="text"
            value={value}
            onChange={(e) => onModelChange(field, e.target.value)}
            placeholder={placeholder}
            autoComplete="off"
            className="flex-1"
          />
          <Button variant="outline" size="icon" className="shrink-0" disabled>
            <Loader2 className="h-4 w-4 animate-spin" />
          </Button>
        </div>
      );
    }

    // Non-Copilot providers: ModelInputWithFetch (fetch button beside the section title)
    return (
      <ModelInputWithFetch
        id={id}
        value={value}
        onChange={(v) => onModelChange(field, v)}
        placeholder={placeholder}
        fetchedModels={fetchedModels}
        isLoading={isFetchingModels}
      />
    );
  };

  return (
    <>
      {/* GitHub Copilot OAuth */}
      {isCopilotPreset && (
        <CopilotAuthSection
          selectedAccountId={selectedGitHubAccountId}
          onAccountSelect={onGitHubAccountSelect}
        />
      )}

      {/* API key input (shown for non-OAuth presets) */}
      {shouldShowApiKey && !usesOAuth && (
        <ApiKeySection
          value={apiKey}
          onChange={onApiKeyChange}
          category={category}
          shouldShowLink={shouldShowApiKeyLink}
          websiteUrl={websiteUrl}
        />
      )}

      {/* Template variable inputs */}
      {templateValueEntries.length > 0 && (
        <div className="space-y-3">
          <FormLabel>
            {t("providerForm.parameterConfig", {
              name: templatePresetName,
              defaultValue: "Parameter Config - {{name}} *",
            })}
          </FormLabel>
          <div className="space-y-4">
            {templateValueEntries.map(([key, config]) => (
              <div key={key} className="space-y-2">
                <FormLabel htmlFor={`template-${key}`}>
                  {config.label}
                </FormLabel>
                <Input
                  id={`template-${key}`}
                  type="text"
                  required
                  value={
                    templateValues[key]?.editorValue ??
                    config.editorValue ??
                    config.defaultValue ??
                    ""
                  }
                  onChange={(e) => onTemplateValueChange(key, e.target.value)}
                  placeholder={config.placeholder || config.label}
                  autoComplete="off"
                />
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Base URL input */}
      {shouldShowSpeedTest && (
        <EndpointField
          id="baseUrl"
          label={t("providerForm.apiEndpoint")}
          value={baseUrl}
          onChange={onBaseUrlChange}
          placeholder={t("providerForm.apiEndpointPlaceholder")}
          hint={
            apiFormat === "openai_responses"
              ? t("providerForm.apiHintResponses")
              : apiFormat === "openai_chat"
                ? t("providerForm.apiHintOAI")
                : t("providerForm.apiHint")
          }
          onManageClick={() => onEndpointModalToggle(true)}
          showFullUrlToggle={true}
          isFullUrl={isFullUrl}
          onFullUrlChange={onFullUrlChange}
        />
      )}

      {/* Endpoint speed-test dialog */}
      {shouldShowSpeedTest && isEndpointModalOpen && (
        <EndpointSpeedTest
          appId="claude"
          providerId={providerId}
          value={baseUrl}
          onChange={onBaseUrlChange}
          initialEndpoints={speedTestEndpoints}
          visible={isEndpointModalOpen}
          onClose={() => onEndpointModalToggle(false)}
          autoSelect={autoSelect}
          onAutoSelectChange={onAutoSelectChange}
          onCustomEndpointsChange={onCustomEndpointsChange}
        />
      )}

      {/* Advanced options (API format, auth field, model mapping) */}
      {shouldShowModelSelector && (
        <Collapsible open={advancedExpanded} onOpenChange={setAdvancedExpanded}>
          <CollapsibleTrigger asChild>
            <Button
              type="button"
              variant={null}
              size="sm"
              className="h-8 gap-1.5 px-0 text-sm font-medium text-foreground hover:opacity-70"
            >
              {advancedExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
              {t("providerForm.advancedOptionsToggle")}
            </Button>
          </CollapsibleTrigger>
          {!advancedExpanded && (
            <p className="text-xs text-muted-foreground mt-1 ml-1">
              {t("providerForm.advancedOptionsHint")}
            </p>
          )}
          <CollapsibleContent className="space-y-4 pt-2">
            {/* API format picker (non-cloud providers only) */}
            {category !== "cloud_provider" && (
              <div className="space-y-2">
                <FormLabel htmlFor="apiFormat">
                  {t("providerForm.apiFormat", { defaultValue: "API Format" })}
                </FormLabel>
                <Select value={apiFormat} onValueChange={onApiFormatChange}>
                  <SelectTrigger id="apiFormat" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="anthropic">
                      {t("providerForm.apiFormatAnthropic", {
                        defaultValue: "Anthropic Messages (Native)",
                      })}
                    </SelectItem>
                    <SelectItem value="openai_chat">
                      {t("providerForm.apiFormatOpenAIChat", {
                        defaultValue:
                          "OpenAI Chat Completions (Requires proxy)",
                      })}
                    </SelectItem>
                    <SelectItem value="openai_responses">
                      {t("providerForm.apiFormatOpenAIResponses", {
                        defaultValue: "OpenAI Responses API (Requires proxy)",
                      })}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("providerForm.apiFormatHint", {
                    defaultValue:
                      "Select the input format for the provider's API",
                  })}
                </p>
              </div>
            )}

            {/* Auth field picker */}
            <div className="space-y-2">
              <FormLabel>
                {t("providerForm.authField", { defaultValue: "Auth Field" })}
              </FormLabel>
              <Select
                value={apiKeyField}
                onValueChange={(v) =>
                  onApiKeyFieldChange(v as ClaudeApiKeyField)
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="ANTHROPIC_AUTH_TOKEN">
                    {t("providerForm.authFieldAuthToken", {
                      defaultValue: "ANTHROPIC_AUTH_TOKEN (Default)",
                    })}
                  </SelectItem>
                  <SelectItem value="ANTHROPIC_API_KEY">
                    {t("providerForm.authFieldApiKey", {
                      defaultValue: "ANTHROPIC_API_KEY",
                    })}
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {t("providerForm.authFieldHint", {
                  defaultValue:
                    "Select the authentication env variable name for the config",
                })}
              </p>
            </div>

            {/* Model mapping */}
            <div className="space-y-1 pt-2 border-t">
              <div className="flex items-center justify-between">
                <FormLabel>{t("providerForm.modelMappingLabel")}</FormLabel>
                {!isCopilotPreset && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleFetchModels}
                    disabled={isFetchingModels}
                    className="h-7 gap-1"
                  >
                    {isFetchingModels ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Download className="h-3.5 w-3.5" />
                    )}
                    {t("providerForm.fetchModels")}
                  </Button>
                )}
              </div>
              <p className="text-xs text-muted-foreground">
                {t("providerForm.modelMappingHint")}
              </p>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {/* Main model */}
              <div className="space-y-2">
                <FormLabel htmlFor="claudeModel">
                  {t("providerForm.anthropicModel", {
                    defaultValue: "Main Model",
                  })}
                </FormLabel>
                {renderModelInput(
                  "claudeModel",
                  claudeModel,
                  "ANTHROPIC_MODEL",
                  t("providerForm.modelPlaceholder", { defaultValue: "" }),
                )}
              </div>

              {/* Reasoning model */}
              <div className="space-y-2">
                <FormLabel htmlFor="reasoningModel">
                  {t("providerForm.anthropicReasoningModel")}
                </FormLabel>
                {renderModelInput(
                  "reasoningModel",
                  reasoningModel,
                  "ANTHROPIC_REASONING_MODEL",
                )}
              </div>

              {/* Default Haiku */}
              <div className="space-y-2">
                <FormLabel htmlFor="claudeDefaultHaikuModel">
                  {t("providerForm.anthropicDefaultHaikuModel", {
                    defaultValue: "Default Haiku Model",
                  })}
                </FormLabel>
                {renderModelInput(
                  "claudeDefaultHaikuModel",
                  defaultHaikuModel,
                  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                  t("providerForm.haikuModelPlaceholder", { defaultValue: "" }),
                )}
              </div>

              {/* Default Sonnet */}
              <div className="space-y-2">
                <FormLabel htmlFor="claudeDefaultSonnetModel">
                  {t("providerForm.anthropicDefaultSonnetModel", {
                    defaultValue: "Default Sonnet Model",
                  })}
                </FormLabel>
                {renderModelInput(
                  "claudeDefaultSonnetModel",
                  defaultSonnetModel,
                  "ANTHROPIC_DEFAULT_SONNET_MODEL",
                  t("providerForm.modelPlaceholder", { defaultValue: "" }),
                )}
              </div>

              {/* Default Opus */}
              <div className="space-y-2">
                <FormLabel htmlFor="claudeDefaultOpusModel">
                  {t("providerForm.anthropicDefaultOpusModel", {
                    defaultValue: "Default Opus Model",
                  })}
                </FormLabel>
                {renderModelInput(
                  "claudeDefaultOpusModel",
                  defaultOpusModel,
                  "ANTHROPIC_DEFAULT_OPUS_MODEL",
                  t("providerForm.modelPlaceholder", { defaultValue: "" }),
                )}
              </div>
            </div>
          </CollapsibleContent>
        </Collapsible>
      )}
    </>
  );
}
