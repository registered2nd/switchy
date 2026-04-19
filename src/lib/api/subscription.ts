import { invoke } from "@tauri-apps/api/core";
import type { SubscriptionQuota } from "@/types/subscription";

export const subscriptionApi = {
  getQuota: (tool: string): Promise<SubscriptionQuota> =>
    invoke("get_subscription_quota", { tool }),
  getQuotaForProvider: (providerId: string): Promise<SubscriptionQuota> =>
    invoke("get_subscription_quota_for_provider", { providerId }),
  getCodingPlanQuota: (
    baseUrl: string,
    apiKey: string,
  ): Promise<SubscriptionQuota> =>
    invoke("get_coding_plan_quota", { baseUrl, apiKey }),
};
