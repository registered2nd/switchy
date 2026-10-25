import { useQuery } from "@tanstack/react-query";
import { accountsApi, type Account } from "@/lib/api/accounts";

export const accountKeys = {
  all: ["accounts"] as const,
  balance: (id: string) => ["accounts", "balance", id] as const,
  planQuota: (id: string) => ["accounts", "planQuota", id] as const,
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
