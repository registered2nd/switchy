import { useQuery } from "@tanstack/react-query";
import { loginAccountApi } from "@/lib/api/loginAccount";
import type { AppId } from "@/lib/api/types";

/**
 * The account a Cursor CLI or Antigravity CLI provider holds: the live login
 * for the current provider (which is then filed into it), the stored one
 * otherwise.
 */
export function useLoginAccountIdentity(
  appId: AppId,
  providerId: string,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["loginAccount", "identity", appId, providerId],
    queryFn: () => loginAccountApi.getIdentity(appId, providerId),
    enabled: enabled && !!providerId,
    staleTime: 30 * 1000,
    refetchOnWindowFocus: true,
    retry: 1,
  });
}
