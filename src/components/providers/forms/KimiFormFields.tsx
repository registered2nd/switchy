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
      {/* Kimi API Key 输入框 */}
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
            defaultValue: "官方供应商使用 kimi login 登录，无需 API Key",
          }),
          thirdParty: t("providerForm.kimiApiKeyAutoFill", {
            defaultValue: "输入 API Key，将自动填充到 config.toml",
          }),
        }}
      />

      {/* Kimi Base URL 输入框 */}
      {shouldShowSpeedTest && (
        <EndpointField
          id="kimiBaseUrl"
          label={t("kimiConfig.apiUrlLabel", { defaultValue: "API 请求地址" })}
          value={kimiBaseUrl}
          onChange={onBaseUrlChange}
          placeholder={t("providerForm.kimiApiEndpointPlaceholder", {
            defaultValue: "例如: https://api.example.com/v1",
          })}
          hint={t("providerForm.kimiApiHint", {
            defaultValue: "写入 config.toml 中当前供应商的 base_url",
          })}
          onManageClick={() => onEndpointModalToggle(true)}
        />
      )}

      {/* Kimi Model Name 输入框 */}
      {shouldShowModelField && onModelNameChange && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label
              htmlFor="kimiModelName"
              className="block text-sm font-medium text-foreground"
            >
              {t("kimiConfig.modelName", { defaultValue: "模型名称" })}
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
              defaultValue: "例如: gpt-4o",
            })}
            fetchedModels={fetchedModels}
            isLoading={isFetchingModels}
          />
          <p className="text-xs text-muted-foreground">
            {modelName.trim()
              ? t("kimiConfig.modelNameHint", {
                  defaultValue:
                    "指定使用的模型，将自动更新 config.toml 的 default_model 与模型别名",
                })
              : t("providerForm.modelHint", {
                  defaultValue: "💡 留空将使用供应商的默认模型",
                })}
          </p>
        </div>
      )}

      {/* 端点测速弹窗 - Kimi */}
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
