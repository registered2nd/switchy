import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { failoverApi } from "@/lib/api/failover";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { extractErrorMessage } from "@/utils/errorUtils";

// ========== Circuit breaker hooks ==========

/**
 * Get provider health
 */
export function useProviderHealth(providerId: string, appType: string) {
  return useQuery({
    queryKey: ["providerHealth", providerId, appType],
    queryFn: () => failoverApi.getProviderHealth(providerId, appType),
    enabled: !!providerId && !!appType,
    refetchInterval: 5000, // Refresh every 5 seconds
    retry: false,
  });
}

/**
 * Reset the circuit breaker
 *
 * After a reset the backend checks whether to switch back to a higher-priority provider,
 * so the provider list and the proxy status are refreshed too.
 */
export function useResetCircuitBreaker() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      providerId,
      appType,
    }: {
      providerId: string;
      appType: string;
    }) => failoverApi.resetCircuitBreaker(providerId, appType),
    onSuccess: (_, variables) => {
      // Refresh health
      queryClient.invalidateQueries({
        queryKey: ["providerHealth", variables.providerId, variables.appType],
      });
      // Refresh the provider list (an automatic switch-back may have happened)
      queryClient.invalidateQueries({
        queryKey: ["providers", variables.appType],
      });
      // Refresh the proxy status (updates active_targets)
      queryClient.invalidateQueries({
        queryKey: ["proxyStatus"],
      });
    },
  });
}

/**
 * Get the circuit breaker config
 */
export function useCircuitBreakerConfig() {
  return useQuery({
    queryKey: ["circuitBreakerConfig"],
    queryFn: () => failoverApi.getCircuitBreakerConfig(),
  });
}

/**
 * Update the circuit breaker config
 */
export function useUpdateCircuitBreakerConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: failoverApi.updateCircuitBreakerConfig,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["circuitBreakerConfig"] });
    },
  });
}

/**
 * Get circuit breaker stats
 */
export function useCircuitBreakerStats(providerId: string, appType: string) {
  return useQuery({
    queryKey: ["circuitBreakerStats", providerId, appType],
    queryFn: () => failoverApi.getCircuitBreakerStats(providerId, appType),
    enabled: !!providerId && !!appType,
    refetchInterval: 5000, // Refresh every 5 seconds
  });
}

// ========== Failover queue hooks ==========

/**
 * Get the failover queue
 */
export function useFailoverQueue(appType: string) {
  return useQuery({
    queryKey: ["failoverQueue", appType],
    queryFn: () => failoverApi.getFailoverQueue(appType),
    enabled: !!appType,
  });
}

/**
 * Get providers that can be added to the queue
 */
export function useAvailableProvidersForFailover(appType: string) {
  return useQuery({
    queryKey: ["availableProvidersForFailover", appType],
    queryFn: () => failoverApi.getAvailableProvidersForFailover(appType),
    enabled: !!appType,
  });
}

/**
 * Add a provider to the failover queue
 */
export function useAddToFailoverQueue() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      appType,
      providerId,
    }: {
      appType: string;
      providerId: string;
    }) => failoverApi.addToFailoverQueue(appType, providerId),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({
        queryKey: ["failoverQueue", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["availableProvidersForFailover", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["providers", variables.appType],
      });
    },
  });
}

/**
 * Remove a provider from the failover queue
 */
export function useRemoveFromFailoverQueue() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      appType,
      providerId,
    }: {
      appType: string;
      providerId: string;
    }) => failoverApi.removeFromFailoverQueue(appType, providerId),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({
        queryKey: ["failoverQueue", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["availableProvidersForFailover", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["providers", variables.appType],
      });
      // Clear this provider's health cache (no health monitoring once it leaves the queue)
      queryClient.invalidateQueries({
        queryKey: ["providerHealth", variables.providerId, variables.appType],
      });
      // Clear this provider's circuit breaker stats cache
      queryClient.invalidateQueries({
        queryKey: [
          "circuitBreakerStats",
          variables.providerId,
          variables.appType,
        ],
      });
    },
  });
}

// ========== Auto-failover switch hooks ==========

/**
 * Get an app's auto-failover switch state
 */
export function useAutoFailoverEnabled(appType: string) {
  return useQuery({
    queryKey: ["autoFailoverEnabled", appType],
    queryFn: () => failoverApi.getAutoFailoverEnabled(appType),
    // Defaults to false (matches the backend)
    placeholderData: false,
  });
}

/**
 * Set an app's auto-failover switch state
 */
export function useSetAutoFailoverEnabled() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({ appType, enabled }: { appType: string; enabled: boolean }) =>
      failoverApi.setAutoFailoverEnabled(appType, enabled),

    // Optimistic update
    onMutate: async ({ appType, enabled }) => {
      await queryClient.cancelQueries({
        queryKey: ["autoFailoverEnabled", appType],
      });
      const previousValue = queryClient.getQueryData<boolean>([
        "autoFailoverEnabled",
        appType,
      ]);

      queryClient.setQueryData(["autoFailoverEnabled", appType], enabled);

      return { previousValue, appType };
    },

    onSuccess: (_data, variables) => {
      const appLabel =
        variables.appType === "claude"
          ? "Claude"
          : variables.appType === "codex"
            ? "Codex"
            : "Gemini";

      toast.success(
        variables.enabled
          ? t("failover.enabled", {
              app: appLabel,
              defaultValue: "{{app}}: Switch automatically on",
            })
          : t("failover.disabled", {
              app: appLabel,
              defaultValue: "{{app}}: Switch automatically off",
            }),
        { closeButton: true },
      );
    },

    // Roll back on error
    onError: (error: Error, _variables, context) => {
      if (context?.previousValue !== undefined) {
        queryClient.setQueryData(
          ["autoFailoverEnabled", context.appType],
          context.previousValue,
        );
      }

      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "Unknown" });
      toast.error(
        t("failover.toggleFailed", {
          detail,
          defaultValue: "Operation failed: {{detail}}",
        }),
      );
    },

    // Refetch whether it succeeded or failed
    onSettled: (_, __, variables) => {
      queryClient.invalidateQueries({
        queryKey: ["autoFailoverEnabled", variables.appType],
      });
      // Turning failover on/off may:
      // - switch to queue P1 right away (current provider changes)
      // - add the current provider to an empty queue (queue contents change)
      queryClient.invalidateQueries({
        queryKey: ["failoverQueue", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["availableProvidersForFailover", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["providers", variables.appType],
      });
      queryClient.invalidateQueries({
        queryKey: ["proxyStatus"],
      });
      queryClient.invalidateQueries({
        queryKey: ["appProxyConfig", variables.appType],
      });
    },
  });
}
