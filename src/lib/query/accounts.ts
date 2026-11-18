import { useQuery } from "@tanstack/react-query";
import { accountsApi, type Account } from "@/lib/api/accounts";
import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";

export const accountKeys = {
  all: ["accounts"] as const,
  balance: (id: string) => ["accounts", "balance", id] as const,
  planQuota: (id: string) => ["accounts", "planQuota", id] as const,
  relayBalance: (appId: AppId, id: string) =>
    ["relayBalance", appId, id] as const,
};

export const useAccountsQuery = () =>
  useQuery({
    queryKey: accountKeys.all,
    queryFn: () => accountsApi.list(),
  });

/** A balance monitor's balance, read again every ten minutes. */
export const useBalanceQuery = (account: Account) =>
  useQuery({
    queryKey: accountKeys.balance(account.id),
    queryFn: () => accountsApi.balance(account.id),
    refetchInterval: 10 * 60 * 1000,
    staleTime: 60 * 1000,
  });

/** A plan's quota windows, read again every five minutes. */
export const usePlanQuotaQuery = (account: Account, enabled: boolean) =>
  useQuery({
    queryKey: [
      ...accountKeys.planQuota(account.id),
      account.plan,
      account.region,
      account.secret,
    ],
    queryFn: () => accountsApi.quota(account.id),
    enabled,
    refetchInterval: 5 * 60 * 1000,
    staleTime: 60 * 1000,
  });

/** A relay card's balance, read again every ten minutes and whenever the
 * card's endpoint, key or balance settings change. */
export const useRelayBalanceQuery = (provider: Provider, appId: AppId) =>
  useQuery({
    queryKey: [
      ...accountKeys.relayBalance(appId, provider.id),
      provider.settingsConfig,
      provider.category,
      provider.meta?.relayBalance,
      provider.meta?.proxyConfig,
    ],
    queryFn: () => accountsApi.relayBalance(provider.id, appId),
    refetchInterval: 10 * 60 * 1000,
    staleTime: 60 * 1000,
  });
