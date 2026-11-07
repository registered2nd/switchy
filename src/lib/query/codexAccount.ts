import { useQuery } from "@tanstack/react-query";
import { codexAccountApi } from "@/lib/api/codexAccount";

/**
 * The ChatGPT account a Codex provider holds — from the live login for the
 * current provider, from the stored one otherwise. Re-read on every provider
 * list change so a switch or a fresh `codex login` shows up.
 */
export function useCodexAccountIdentity(providerId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["codexAccount", "identity", providerId],
    queryFn: () => codexAccountApi.getIdentity(providerId),
    enabled: enabled && !!providerId,
    staleTime: 30 * 1000,
    refetchOnWindowFocus: true,
    retry: 1,
  });
}
