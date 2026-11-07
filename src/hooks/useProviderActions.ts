import { useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { providersApi, openclawApi, type AppId } from "@/lib/api";
import type {
  Provider,
  UsageScript,
  OpenClawProviderConfig,
  OpenClawDefaultModel,
} from "@/types";
import type { OpenClawSuggestedDefaults } from "@/config/openclawProviderPresets";
import {
  useAddProviderMutation,
  useUpdateProviderMutation,
  useDeleteProviderMutation,
  useSwitchProviderMutation,
} from "@/lib/query";
import { extractErrorMessage } from "@/utils/errorUtils";
import { openclawKeys } from "@/hooks/useOpenClaw";

/**
 * Hook for managing provider actions (add, update, delete, switch)
 * Extracts business logic from App.tsx
 */
export function useProviderActions(activeApp: AppId, isProxyRunning?: boolean) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const addProviderMutation = useAddProviderMutation(activeApp);
  const updateProviderMutation = useUpdateProviderMutation(activeApp);
  const deleteProviderMutation = useDeleteProviderMutation(activeApp);
  const switchProviderMutation = useSwitchProviderMutation(activeApp);

  // Add provider
  const addProvider = useCallback(
    async (
      provider: Omit<Provider, "id"> & {
        providerKey?: string;
        suggestedDefaults?: OpenClawSuggestedDefaults;
        addToLive?: boolean;
      },
    ) => {
      await addProviderMutation.mutateAsync(provider);

      // OpenClaw: register models to allowlist after adding provider
      if (activeApp === "openclaw" && provider.suggestedDefaults) {
        const { model, modelCatalog } = provider.suggestedDefaults;
        let modelsRegistered = false;

        try {
          // 1. Merge model catalog (allowlist)
          if (modelCatalog && Object.keys(modelCatalog).length > 0) {
            const existingCatalog = (await openclawApi.getModelCatalog()) || {};
            const mergedCatalog = { ...existingCatalog, ...modelCatalog };
            await openclawApi.setModelCatalog(mergedCatalog);
            await queryClient.invalidateQueries({
              queryKey: openclawKeys.health,
            });
            modelsRegistered = true;
          }

          // 2. Set default model (only if not already set)
          if (model) {
            const existingDefault = await openclawApi.getDefaultModel();
            if (!existingDefault?.primary) {
              await openclawApi.setDefaultModel(model);
              await queryClient.invalidateQueries({
                queryKey: openclawKeys.health,
              });
            }
          }

          // Show success toast if models were registered
          if (modelsRegistered) {
            toast.success(
              t("notifications.openclawModelsRegistered", {
                defaultValue: "Models have been registered to /model list",
              }),
              { closeButton: true },
            );
          }
        } catch (error) {
          // Log warning but don't block main flow - provider config is already saved
          console.warn(
            "[OpenClaw] Failed to register models to allowlist:",
            error,
          );
        }
      }
    },
    [addProviderMutation, activeApp, queryClient, t],
  );

  // Update provider
  const updateProvider = useCallback(
    async (provider: Provider, originalId?: string) => {
      await updateProviderMutation.mutateAsync({ provider, originalId });

      // Update the tray menu (a failure does not affect the main action)
      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after updating provider",
          trayError,
        );
      }
    },
    [updateProviderMutation],
  );

  // Switch provider
  const switchProvider = useCallback(
    async (provider: Provider) => {
      const isCopilotProvider =
        activeApp === "claude" &&
        provider.meta?.providerType === "github_copilot";

      // Determine why this provider requires the proxy
      let proxyRequiredReason: string | null = null;
      if (!isProxyRunning && provider.category !== "official") {
        if (isCopilotProvider) {
          proxyRequiredReason = t("notifications.proxyReasonCopilot", {
            defaultValue: "uses GitHub Copilot as a Claude provider",
          });
        } else if (
          provider.meta?.apiFormat === "openai_chat" &&
          activeApp === "claude"
        ) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
            defaultValue: "uses OpenAI Chat API format",
          });
        } else if (
          provider.meta?.apiFormat === "openai_responses" &&
          activeApp === "claude"
        ) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIResponses", {
            defaultValue: "uses OpenAI Responses API format",
          });
        } else if (
          provider.meta?.isFullUrl &&
          (activeApp === "claude" || activeApp === "codex")
        ) {
          proxyRequiredReason = t("notifications.proxyReasonFullUrl", {
            defaultValue: "has full URL connection mode enabled",
          });
        }
      }

      if (proxyRequiredReason) {
        toast.warning(
          t("notifications.proxyRequiredForSwitch", {
            reason: proxyRequiredReason,
            defaultValue:
              "This provider {{reason}}, requires the proxy service to work properly. Start the proxy first.",
          }),
        );
      }

      try {
        const result = await switchProviderMutation.mutateAsync(provider.id);

        // Surface tagged warnings from SwitchResult (AC-2.3 / AC-4.2 / AC-4.3).
        if (result?.warnings?.length) {
          let showedGenericBackfill = false;
          for (const tag of result.warnings) {
            if (tag.startsWith("credential_swap_failed:")) {
              const id = tag.slice("credential_swap_failed:".length);
              toast.warning(
                t("claudeAccount.swap.warning.failed_swap", {
                  id,
                  defaultValue: `Credential swap failed for ${id}. The switch was saved; retry after closing Claude Code.`,
                }),
                { duration: 6000 },
              );
            } else if (tag.startsWith("credential_mirror_failed:")) {
              const rest = tag.slice("credential_mirror_failed:".length);
              const idx = rest.lastIndexOf(":");
              const reason = idx === -1 ? "unreachable" : rest.slice(idx + 1);
              if (reason === "locked") {
                toast.warning(
                  t("claudeAccount.swap.warning.locked", {
                    path: "WSL mirror",
                    defaultValue: `Swap partially applied: WSL mirror is locked. Close Claude Code and retry.`,
                  }),
                  { duration: 6000 },
                );
              } else {
                toast.warning(
                  t("claudeAccount.swap.warning.mirror_skipped", {
                    reason,
                    defaultValue: `WSL mirror skipped — ${reason}`,
                  }),
                  { duration: 6000 },
                );
              }
            } else if (tag.startsWith("backfill_account_mismatch:")) {
              const id = tag.slice("backfill_account_mismatch:".length);
              toast.warning(
                t("codexAccount.warning.account_mismatch", {
                  id,
                  defaultValue: `The Codex login that was live belonged to a different account than "${id}" holds; "${id}" keeps its saved login.`,
                }),
                { duration: 6000 },
              );
            } else if (tag.startsWith("backfill_rehomed:")) {
              const id = tag.slice("backfill_rehomed:".length);
              toast.info(
                t("codexAccount.warning.rehomed", {
                  id,
                  defaultValue: `That login was saved to "${id}", the provider that holds that account.`,
                }),
                { duration: 6000 },
              );
            } else if (
              tag.startsWith("backfill_failed:") &&
              !showedGenericBackfill
            ) {
              showedGenericBackfill = true;
              toast.warning(
                t("notifications.backfillWarning", {
                  defaultValue:
                    "Switched successfully, but failed to save changes back to the previous provider",
                }),
                { duration: 5000 },
              );
            }
          }
        }

        // Show a different success toast depending on the provider type
        if (
          !proxyRequiredReason &&
          activeApp === "claude" &&
          provider.category !== "official" &&
          (isCopilotProvider ||
            provider.meta?.apiFormat === "openai_chat" ||
            provider.meta?.apiFormat === "openai_responses")
        ) {
          // OpenAI format provider: show proxy hint (skip if warning already shown)
          toast.info(
            isCopilotProvider
              ? t("notifications.copilotProxyHint")
              : t("notifications.openAIFormatHint"),
            {
              duration: 5000,
              closeButton: true,
            },
          );
        } else {
          // Regular provider: show switch success
          // OpenCode/OpenClaw: show "added to config" message instead of "switched"
          const isMultiProviderApp =
            activeApp === "opencode" || activeApp === "openclaw";
          const messageKey = isMultiProviderApp
            ? "notifications.addToConfigSuccess"
            : "notifications.switchSuccess";
          const defaultMessage = isMultiProviderApp
            ? "Added to config"
            : "Switch successful!";

          toast.success(t(messageKey, { defaultValue: defaultMessage }), {
            closeButton: true,
          });
        }
      } catch {
        // The mutation handles the error toast
      }
    },
    [switchProviderMutation, activeApp, isProxyRunning, t],
  );

  // Delete provider
  const deleteProvider = useCallback(
    async (id: string) => {
      await deleteProviderMutation.mutateAsync(id);
    },
    [deleteProviderMutation],
  );

  // Save usage script
  const saveUsageScript = useCallback(
    async (provider: Provider, script: UsageScript) => {
      try {
        const updatedProvider: Provider = {
          ...provider,
          meta: {
            ...provider.meta,
            usage_script: script,
          },
        };

        await providersApi.update(updatedProvider, activeApp);
        await queryClient.invalidateQueries({
          queryKey: ["providers", activeApp],
        });
        // After saving the usage script, also invalidate this provider's usage query cache
        // so the main list re-queries with the new config instead of the cache from the test run
        await queryClient.invalidateQueries({
          queryKey: ["usage", provider.id, activeApp],
        });
        toast.success(
          t("provider.usageSaved", {
            defaultValue: "Usage query configuration saved",
          }),
          { closeButton: true },
        );
      } catch (error) {
        const detail =
          extractErrorMessage(error) ||
          t("provider.usageSaveFailed", {
            defaultValue: "Failed to save usage query configuration",
          });
        toast.error(detail);
      }
    },
    [activeApp, queryClient, t],
  );

  // Set provider as default model (OpenClaw only)
  const setAsDefaultModel = useCallback(
    async (provider: Provider) => {
      const config = provider.settingsConfig as OpenClawProviderConfig;
      if (!config.models || config.models.length === 0) {
        toast.error(
          t("notifications.openclawNoModels", {
            defaultValue: "No models configured",
          }),
        );
        return;
      }

      const model: OpenClawDefaultModel = {
        primary: `${provider.id}/${config.models[0].id}`,
        fallbacks: config.models.slice(1).map((m) => `${provider.id}/${m.id}`),
      };

      try {
        await openclawApi.setDefaultModel(model);
        await queryClient.invalidateQueries({
          queryKey: openclawKeys.defaultModel,
        });
        await queryClient.invalidateQueries({
          queryKey: openclawKeys.health,
        });
        toast.success(
          t("notifications.openclawDefaultModelSet", {
            defaultValue: "Set as default model",
          }),
          { closeButton: true },
        );
      } catch (error) {
        const detail =
          extractErrorMessage(error) ||
          t("notifications.openclawDefaultModelSetFailed", {
            defaultValue: "Failed to set default model",
          });
        toast.error(detail);
      }
    },
    [queryClient, t],
  );

  return {
    addProvider,
    updateProvider,
    switchProvider,
    deleteProvider,
    saveUsageScript,
    setAsDefaultModel,
    isLoading:
      addProviderMutation.isPending ||
      updateProviderMutation.isPending ||
      deleteProviderMutation.isPending ||
      switchProviderMutation.isPending,
  };
}
