import { useQuery } from "@tanstack/react-query";
import { subscriptionApi } from "@/lib/api/subscription";
import type { AppId } from "@/lib/api/types";

const REFETCH_INTERVAL = 5 * 60 * 1000; // 5 minutes
// A card that leaves the screen keeps its last reading this long, so going
// back to its tab shows it at once while a fresh one is fetched.
const KEEP_READING = 30 * 60 * 1000;

/** `liveOwner` is the provider in use, so a switch never shows the previous account's figures. */
export function useSubscriptionQuota(
  appId: AppId,
  enabled: boolean,
  liveOwner?: string,
) {
  return useQuery({
    queryKey: ["subscription", "quota", appId, liveOwner ?? ""],
    queryFn: () => subscriptionApi.getQuota(appId),
    gcTime: KEEP_READING,
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
    gcTime: KEEP_READING,
    enabled:
      enabled &&
      !!providerId &&
      ["claude", "codex", "cursor", "antigravity"].includes(appId),
    refetchInterval: REFETCH_INTERVAL,
    refetchOnWindowFocus: true,
    staleTime: REFETCH_INTERVAL,
    retry: 1,
  });
}
