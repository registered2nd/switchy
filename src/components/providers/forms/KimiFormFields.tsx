import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { Download, Loader2 } from "lucide-react";
import EndpointSpeedTest from "./EndpointSpeedTest";
import { ApiKeySection, EndpointField, ModelInputWithFetch } from "./shared";
import {
  fetchModelsForConfig,
  showFetchModelsError,
  type FetchedModel,
} from "@/lib/api/model-fetch";
import type { ProviderCategory } from "@/types";

interface EndpointCandidate {
  url: string;
}

interface KimiFormFieldsProps {
  providerId?: string;
  // API Key
  kimiApiKey: string;
  onApiKeyChange: (key: string) => void;
  category?: ProviderCategory;
  shouldShowApiKeyLink: boolean;
  websiteUrl: string;

  // Base URL
  shouldShowSpeedTest: boolean;
  kimiBaseUrl: string;
  onBaseUrlChange: (url: string) => void;
  isEndpointModalOpen: boolean;
  onEndpointModalToggle: (open: boolean) => void;
  onCustomEndpointsChange?: (endpoints: string[]) => void;
  autoSelect: boolean;
  onAutoSelectChange: (checked: boolean) => void;

  // Model Name
  shouldShowModelField?: boolean;
  modelName?: string;
  onModelNameChange?: (model: string) => void;

  // Speed Test Endpoints
  speedTestEndpoints: EndpointCandidate[];
}

export function KimiFormFields({
  providerId,
  kimiApiKey,
  onApiKeyChange,
  category,
  shouldShowApiKeyLink,
  websiteUrl,
  shouldShowSpeedTest,
  kimiBaseUrl,
  onBaseUrlChange,
  isEndpointModalOpen,
  onEndpointModalToggle,
  onCustomEndpointsChange,
  autoSelect,
  onAutoSelectChange,
  shouldShowModelField = true,
  modelName = "",
  onModelNameChange,
  speedTestEndpoints,
}: KimiFormFieldsProps) {
  const { t } = useTranslation();

  const [fetchedModels, setFetchedModels] = useState<FetchedModel[]>([]);
  const [isFetchingModels, setIsFetchingModels] = useState(false);

  const handleFetchModels = useCallback(() => {
    if (!kimiBaseUrl || !kimiApiKey) {
      showFetchModelsError(null, t, {
        hasApiKey: !!kimiApiKey,
        hasBaseUrl: !!kimiBaseUrl,
      });
      return;
    }
    setIsFetchingModels(true);
    fetchModelsForConfig(kimiBaseUrl, kimiApiKey, false)
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
  }, [kimiBaseUrl, kimiApiKey, t]);

  return (
    <>
      {/* Kimi API key input */}
      <ApiKeySection
        id="kimiApiKey"
        label="API Key"
        value={kimiApiKey}
        onChange={onApiKeyChange}
        category={category}
        shouldShowLink={shouldShowApiKeyLink}
        websiteUrl={websiteUrl}
        placeholder={{
          official: t("providerForm.kimiOfficialNoApiKey", {
            defaultValue:
              "Official provider signs in with `kimi login`; no API Key needed",
          }),
          thirdParty: t("providerForm.kimiApiKeyAutoFill", {
            defaultValue: "Enter API Key; it is written into config.toml",
          }),
        }}
      />

      {/* Kimi Base URL input */}
      {shouldShowSpeedTest && (
        <EndpointField
          id="kimiBaseUrl"
          label={t("kimiConfig.apiUrlLabel", {
            defaultValue: "API Request URL",
          })}
          value={kimiBaseUrl}
          onChange={onBaseUrlChange}
          placeholder={t("providerForm.kimiApiEndpointPlaceholder", {
            defaultValue: "e.g., https://api.example.com/v1",
          })}
          hint={t("providerForm.kimiApiHint", {
            defaultValue:
              "Written as base_url of the active provider in config.toml",
          })}
          onManageClick={() => onEndpointModalToggle(true)}
        />
      )}

      {/* Kimi model name input */}
      {shouldShowModelField && onModelNameChange && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label
              htmlFor="kimiModelName"
              className="block text-sm font-medium text-foreground"
            >
              {t("kimiConfig.modelName", { defaultValue: "Model Name" })}
            </label>
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
          </div>
          <ModelInputWithFetch
            id="kimiModelName"
            value={modelName}
            onChange={(v) => onModelNameChange!(v)}
            placeholder={t("kimiConfig.modelNamePlaceholder", {
              defaultValue: "e.g., gpt-4o",
            })}
            fetchedModels={fetchedModels}
            isLoading={isFetchingModels}
          />
          <p className="text-xs text-muted-foreground">
            {modelName.trim()
              ? t("kimiConfig.modelNameHint", {
                  defaultValue:
                    "Sets default_model and the model alias in config.toml",
                })
              : t("providerForm.modelHint", {
                  defaultValue:
                    "💡 Leave blank to use provider's default model",
                })}
          </p>
        </div>
      )}

      {/* Endpoint speed-test dialog - Kimi */}
      {shouldShowSpeedTest && isEndpointModalOpen && (
        <EndpointSpeedTest
          appId="kimi"
          providerId={providerId}
          value={kimiBaseUrl}
          onChange={onBaseUrlChange}
          initialEndpoints={speedTestEndpoints}
          visible={isEndpointModalOpen}
          onClose={() => onEndpointModalToggle(false)}
          autoSelect={autoSelect}
          onAutoSelectChange={onAutoSelectChange}
          onCustomEndpointsChange={onCustomEndpointsChange}
        />
      )}
    </>
  );
}
