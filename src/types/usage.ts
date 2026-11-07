// Usage statistics types

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
}

export interface RequestLog {
  requestId: string;
  providerId: string;
  providerName?: string;
  appType: string;
  model: string;
  requestModel?: string;
  costMultiplier: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  inputCostUsd: string;
  outputCostUsd: string;
  cacheReadCostUsd: string;
  cacheCreationCostUsd: string;
  totalCostUsd: string;
  isStreaming: boolean;
  latencyMs: number;
  firstTokenMs?: number;
  durationMs?: number;
  statusCode: number;
  errorMessage?: string;
  createdAt: number;
}

export interface PaginatedLogs {
  data: RequestLog[];
  total: number;
  page: number;
  pageSize: number;
}

export interface ModelPricing {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

export interface UsageSummary {
  totalRequests: number;
  totalCost: string;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  successRate: number;
}

export interface DailyStats {
  date: string;
  requestCount: number;
  totalCost: string;
  totalTokens: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
}

export interface ProviderStats {
  providerId: string;
  providerName: string;
  appType: string;
  /** The signed-in account behind a pooled provider. */
  accountEmail?: string | null;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  successRate: number;
  /** Requests the account refused with 429. */
  limitedCount: number;
  avgLatencyMs: number;
  /** Unix seconds of the newest logged request. */
  lastUsedAt?: number | null;
}

export type SwitchReason =
  | "manual"
  | "failover"
  | "limit"
  | "signed_out"
  | "rotation"
  | "recovered";

/** A change of the account serving an app. */
export interface AccountSwitch {
  id: number;
  appType: string;
  fromProviderId?: string | null;
  fromProviderName?: string | null;
  fromAccount?: string | null;
  toProviderId: string;
  toProviderName?: string | null;
  toAccount?: string | null;
  reason: SwitchReason;
  detail?: string | null;
  /** Unix seconds. */
  createdAt: number;
}

export interface ModelStats {
  model: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
}

export interface LogFilters {
  appType?: string;
  providerName?: string;
  model?: string;
  statusCode?: number;
  startDate?: number;
  endDate?: number;
}

export interface ProviderLimitStatus {
  providerId: string;
  dailyUsage: string;
  dailyLimit?: string;
  dailyExceeded: boolean;
  monthlyUsage: string;
  monthlyLimit?: string;
  monthlyExceeded: boolean;
}

export type TimeRange = "1d" | "7d" | "30d";

export interface StatsFilters {
  timeRange: TimeRange;
  providerId?: string;
  appType?: string;
}
