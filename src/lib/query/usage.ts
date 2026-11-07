import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { usageApi } from "@/lib/api/usage";
import type { LogFilters } from "@/types/usage";

const DEFAULT_REFETCH_INTERVAL_MS = 30000;

type UsageQueryOptions = {
  refetchInterval?: number | false;
  refetchIntervalInBackground?: boolean;
};

type RequestLogsTimeMode = "rolling" | "fixed";

type RequestLogsQueryArgs = {
  filters: LogFilters;
  timeMode: RequestLogsTimeMode;
  page?: number;
  pageSize?: number;
  rollingWindowSeconds?: number;
  options?: UsageQueryOptions;
};

type RequestLogsKey = {
  timeMode: RequestLogsTimeMode;
  rollingWindowSeconds?: number;
  appType?: string;
  providerName?: string;
  model?: string;
  statusCode?: number;
  startDate?: number;
  endDate?: number;
};

// Query keys
export const usageKeys = {
  all: ["usage"] as const,
  summary: (days: number) => [...usageKeys.all, "summary", days] as const,
  trends: (days: number) => [...usageKeys.all, "trends", days] as const,
  providerStats: (days: number) =>
    [...usageKeys.all, "provider-stats", days] as const,
  modelStats: (days: number) =>
    [...usageKeys.all, "model-stats", days] as const,
  switches: (days: number) => [...usageKeys.all, "switches", days] as const,
  logs: (key: RequestLogsKey, page: number, pageSize: number) =>
    [
      ...usageKeys.all,
      "logs",
      key.timeMode,
      key.rollingWindowSeconds ?? 0,
      key.appType ?? "",
      key.providerName ?? "",
      key.model ?? "",
      key.statusCode ?? -1,
      key.startDate ?? 0,
      key.endDate ?? 0,
      page,
      pageSize,
    ] as const,
  detail: (requestId: string) =>
    [...usageKeys.all, "detail", requestId] as const,
  pricing: () => [...usageKeys.all, "pricing"] as const,
  limits: (providerId: string, appType: string) =>
    [...usageKeys.all, "limits", providerId, appType] as const,
};

const getWindow = (days: number) => {
  const endDate = Math.floor(Date.now() / 1000);
  const startDate = endDate - days * 24 * 60 * 60;
  return { startDate, endDate };
};

// Hooks
export function useUsageSummary(days: number, options?: UsageQueryOptions) {
  return useQuery({
    queryKey: usageKeys.summary(days),
    queryFn: () => {
      const { startDate, endDate } = getWindow(days);
      return usageApi.getUsageSummary(startDate, endDate);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS, // Auto-refresh every 30 seconds
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false, // No refresh in the background
  });
}

export function useUsageTrends(days: number, options?: UsageQueryOptions) {
  return useQuery({
    queryKey: usageKeys.trends(days),
    queryFn: () => {
      const { startDate, endDate } = getWindow(days);
      return usageApi.getUsageTrends(startDate, endDate);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS, // Auto-refresh every 30 seconds
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false,
  });
}

export function useProviderStats(days: number, options?: UsageQueryOptions) {
  return useQuery({
    queryKey: usageKeys.providerStats(days),
    queryFn: () => {
      const { startDate, endDate } = getWindow(days);
      return usageApi.getProviderStats(startDate, endDate);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS, // Auto-refresh every 30 seconds
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false,
  });
}

export function useModelStats(days: number, options?: UsageQueryOptions) {
  return useQuery({
    queryKey: usageKeys.modelStats(days),
    queryFn: () => {
      const { startDate, endDate } = getWindow(days);
      return usageApi.getModelStats(startDate, endDate);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS, // Auto-refresh every 30 seconds
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false,
  });
}

export function useAccountSwitches(days: number, options?: UsageQueryOptions) {
  return useQuery({
    queryKey: usageKeys.switches(days),
    queryFn: () => {
      const { startDate } = getWindow(days);
      return usageApi.getAccountSwitches(undefined, startDate, 200);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS,
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false,
  });
}

const getRollingRange = (windowSeconds: number) => {
  const endDate = Math.floor(Date.now() / 1000);
  const startDate = endDate - windowSeconds;
  return { startDate, endDate };
};

export function useRequestLogs({
  filters,
  timeMode,
  page = 0,
  pageSize = 20,
  rollingWindowSeconds = 24 * 60 * 60,
  options,
}: RequestLogsQueryArgs) {
  const key: RequestLogsKey = {
    timeMode,
    rollingWindowSeconds:
      timeMode === "rolling" ? rollingWindowSeconds : undefined,
    appType: filters.appType,
    providerName: filters.providerName,
    model: filters.model,
    statusCode: filters.statusCode,
    startDate: timeMode === "fixed" ? filters.startDate : undefined,
    endDate: timeMode === "fixed" ? filters.endDate : undefined,
  };

  return useQuery({
    queryKey: usageKeys.logs(key, page, pageSize),
    queryFn: () => {
      const effectiveFilters =
        timeMode === "rolling"
          ? { ...filters, ...getRollingRange(rollingWindowSeconds) }
          : filters;
      return usageApi.getRequestLogs(effectiveFilters, page, pageSize);
    },
    refetchInterval: options?.refetchInterval ?? DEFAULT_REFETCH_INTERVAL_MS, // Auto-refresh every 30 seconds
    refetchIntervalInBackground: options?.refetchIntervalInBackground ?? false,
  });
}

export function useRequestDetail(requestId: string) {
  return useQuery({
    queryKey: usageKeys.detail(requestId),
    queryFn: () => usageApi.getRequestDetail(requestId),
    enabled: !!requestId,
  });
}

export function useModelPricing() {
  return useQuery({
    queryKey: usageKeys.pricing(),
    queryFn: usageApi.getModelPricing,
  });
}

export function useProviderLimits(providerId: string, appType: string) {
  return useQuery({
    queryKey: usageKeys.limits(providerId, appType),
    queryFn: () => usageApi.checkProviderLimits(providerId, appType),
    enabled: !!providerId && !!appType,
  });
}

export function useUpdateModelPricing() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (params: {
      modelId: string;
      displayName: string;
      inputCost: string;
      outputCost: string;
      cacheReadCost: string;
      cacheCreationCost: string;
    }) =>
      usageApi.updateModelPricing(
        params.modelId,
        params.displayName,
        params.inputCost,
        params.outputCost,
        params.cacheReadCost,
        params.cacheCreationCost,
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: usageKeys.pricing() });
    },
  });
}

export function useDeleteModelPricing() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (modelId: string) => usageApi.deleteModelPricing(modelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: usageKeys.pricing() });
    },
  });
}
