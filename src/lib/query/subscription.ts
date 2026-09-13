import { useQuery } from "@tanstack/react-query";
import { subscriptionApi } from "@/lib/api/subscription";
import type { AppId } from "@/lib/api/types";

const REFETCH_INTERVAL = 5 * 60 * 1000; // 5 minutes

export function useSubscriptionQuota(appId: AppId, enabled: boolean) {
  return useQuery({
    queryKey: ["subscription", "quota", appId],
    queryFn: () => subscriptionApi.getQuota(appId),
    enabled: enabled && ["claude", "codex", "gemini"].includes(appId),
    refetchInterval: REFETCH_INTERVAL,
    refetchOnWindowFocus: true,
    staleTime: REFETCH_INTERVAL,
    retry: 1,
  });
}

/**
 * Per-provider subscription quota, keyed on provider id so each Official
 * card shows its own account's usage: Claude reads the captured snapshot,
 * Codex the login stored in the provider itself.
 */
export function useSubscriptionQuotaForProvider(
  appId: AppId,
  providerId: string,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["subscription", "quota", "provider", appId, providerId],
    queryFn: () => subscriptionApi.getQuotaForProvider(appId, providerId),
    enabled: enabled && !!providerId && ["claude", "codex"].includes(appId),
    refetchInterval: REFETCH_INTERVAL,
    refetchOnWindowFocus: true,
    staleTime: REFETCH_INTERVAL,
    retry: 1,
  });
}
