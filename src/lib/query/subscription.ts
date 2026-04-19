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
 * Per-provider Claude subscription quota, keyed on provider id so each
 * captured Official card shows its own account's usage. See BACKLOG #5.
 */
export function useSubscriptionQuotaForProvider(
  providerId: string,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["subscription", "quota", "provider", providerId],
    queryFn: () => subscriptionApi.getQuotaForProvider(providerId),
    enabled: enabled && !!providerId,
    refetchInterval: REFETCH_INTERVAL,
    refetchOnWindowFocus: true,
    staleTime: REFETCH_INTERVAL,
    retry: 1,
  });
}
