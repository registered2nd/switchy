import {
  useQuery,
  type UseQueryResult,
  keepPreviousData,
} from "@tanstack/react-query";
import {
  providersApi,
  settingsApi,
  usageApi,
  type AppId,
} from "@/lib/api";
import type {
  Provider,
  Settings,
  UsageResult,
} from "@/types";

const sortProviders = (
  providers: Record<string, Provider>,
): Record<string, Provider> => {
  const sortedEntries = Object.values(providers)
    .sort((a, b) => {
      const indexA = a.sortIndex ?? Number.MAX_SAFE_INTEGER;
      const indexB = b.sortIndex ?? Number.MAX_SAFE_INTEGER;
      if (indexA !== indexB) {
        return indexA - indexB;
      }

      const timeA = a.createdAt ?? 0;
      const timeB = b.createdAt ?? 0;
      if (timeA === timeB) {
        return a.name.localeCompare(b.name, "zh-CN");
      }
      return timeA - timeB;
    })
    .map((provider) => [provider.id, provider] as const);

  return Object.fromEntries(sortedEntries);
};

export interface ProvidersQueryData {
  providers: Record<string, Provider>;
  currentProviderId: string;
}

export interface UseProvidersQueryOptions {
  isProxyRunning?: boolean; // Whether the proxy service is running
}

export const useProvidersQuery = (
  appId: AppId,
  options?: UseProvidersQueryOptions,
): UseQueryResult<ProvidersQueryData> => {
  const { isProxyRunning = false } = options || {};

  return useQuery({
    queryKey: ["providers", appId],
    placeholderData: keepPreviousData,
    // While the proxy service runs, refresh the provider list every 10 seconds
    // so it picks up proxy targets the backend circuit breaker disabled automatically
    refetchInterval: isProxyRunning ? 10000 : false,
    queryFn: async () => {
      let providers: Record<string, Provider> = {};
      let currentProviderId = "";

      try {
        providers = await providersApi.getAll(appId);
      } catch (error) {
        console.error("Failed to get the provider list:", error);
      }

      try {
        currentProviderId = await providersApi.getCurrent(appId);
      } catch (error) {
        console.error("Failed to get the current provider:", error);
      }

      return {
        providers: sortProviders(providers),
        currentProviderId,
      };
    },
  });
};

export const useSettingsQuery = (): UseQueryResult<Settings> => {
  return useQuery({
    queryKey: ["settings"],
    queryFn: async () => settingsApi.get(),
  });
};

export interface UseUsageQueryOptions {
  enabled?: boolean;
  autoQueryInterval?: number; // Auto-query interval in minutes, 0 disables
}

export const useUsageQuery = (
  providerId: string,
  appId: AppId,
  options?: UseUsageQueryOptions,
) => {
  const { enabled = true, autoQueryInterval = 0 } = options || {};

  // staleTime: the auto-refresh interval if there is one, otherwise 5 minutes
  // so switching app pages does not trigger the query again
  const staleTime =
    autoQueryInterval > 0
      ? autoQueryInterval * 60 * 1000 // Same as the refresh interval
      : 5 * 60 * 1000; // Default 5 minutes

  const query = useQuery<UsageResult>({
    queryKey: ["usage", providerId, appId],
    queryFn: async () => usageApi.query(providerId, appId),
    enabled: enabled && !!providerId,
    refetchInterval:
      autoQueryInterval > 0
        ? Math.max(autoQueryInterval, 1) * 60 * 1000 // At least 1 minute
        : false,
    refetchIntervalInBackground: true, // Keep polling in the background
    refetchOnWindowFocus: false,
    retry: false,
    staleTime, // Computed cache time
    gcTime: 10 * 60 * 1000, // Keep the cache 10 minutes (after unmount)
  });

  return {
    ...query,
    lastQueriedAt: query.dataUpdatedAt || null,
  };
};

