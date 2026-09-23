/**
 * GitHub Copilot OAuth API
 *
 * API functions for the GitHub Copilot OAuth device code flow.
 * Supports multiple accounts.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * GitHub device code response
 */
export interface CopilotDeviceCodeResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

/**
 * GitHub account info (public)
 */
export interface GitHubAccount {
  /** GitHub user ID (unique) */
  id: string;
  /** GitHub username */
  login: string;
  /** Avatar URL */
  avatar_url: string | null;
  /** Auth timestamp (Unix seconds) */
  authenticated_at: number;
}

/**
 * Copilot auth status (multi-account)
 */
export interface CopilotAuthStatus {
  /** Authenticated (any account) - backward compatible */
  authenticated: boolean;
  /** Default account ID */
  default_account_id: string | null;
  /** Status message when migrating legacy auth data failed */
  migration_error?: string | null;
  /** Username of the first account - backward compatible */
  username: string | null;
  /** Copilot token expiry - backward compatible */
  expires_at: number | null;
  /** All authenticated accounts */
  accounts: GitHubAccount[];
}

/**
 * Start the GitHub OAuth device code flow
 *
 * @returns device code response with the user code and verification URL
 */
export async function copilotStartDeviceFlow(): Promise<CopilotDeviceCodeResponse> {
  return invoke<CopilotDeviceCodeResponse>("copilot_start_device_flow");
}

/**
 * Poll for the OAuth token
 *
 * Polls GitHub with the device code until the user finishes authorizing.
 *
 * @param deviceCode - device code
 * @returns true when authenticated, false while still waiting for the user
 */
export async function copilotPollForAuth(deviceCode: string): Promise<boolean> {
  return invoke<boolean>("copilot_poll_for_auth", {
    deviceCode,
  });
}

/**
 * Get the Copilot auth status
 *
 * @returns auth status: whether authenticated, username and expiry
 */
export async function copilotGetAuthStatus(): Promise<CopilotAuthStatus> {
  return invoke<CopilotAuthStatus>("copilot_get_auth_status");
}

/**
 * Sign out of Copilot
 */
export async function copilotLogout(): Promise<void> {
  return invoke("copilot_logout");
}

/**
 * Whether authenticated
 *
 * @returns true when authenticated
 */
export async function copilotIsAuthenticated(): Promise<boolean> {
  return invoke<boolean>("copilot_is_authenticated");
}

/**
 * Copilot available model
 */
export interface CopilotModel {
  id: string;
  name: string;
  vendor: string;
  model_picker_enabled: boolean;
}

/**
 * Get a valid Copilot token
 *
 * Internal, used for proxied requests.
 *
 * @returns Copilot Token
 */
export async function copilotGetToken(): Promise<string> {
  return invoke<string>("copilot_get_token");
}

/**
 * Get the Copilot available models
 *
 * @returns available models
 */
export async function copilotGetModels(): Promise<CopilotModel[]> {
  return invoke<CopilotModel[]>("copilot_get_models");
}

/**
 * Quota details
 */
export interface QuotaDetail {
  entitlement: number;
  remaining: number;
  percent_remaining: number;
  unlimited: boolean;
}

/**
 * Quota snapshot
 */
export interface QuotaSnapshots {
  chat: QuotaDetail;
  completions: QuotaDetail;
  premium_interactions: QuotaDetail;
}

/**
 * Copilot usage response
 */
export interface CopilotUsageResponse {
  copilot_plan: string;
  quota_reset_date: string;
  quota_snapshots: QuotaSnapshots;
}

/**
 * Get Copilot usage info
 *
 * @returns usage info: plan type, reset date and quota snapshots
 */
export async function copilotGetUsage(): Promise<CopilotUsageResponse> {
  return invoke<CopilotUsageResponse>("copilot_get_usage");
}

// ==================== Multi-account API ====================

/**
 * List all authenticated GitHub accounts
 *
 * @returns accounts
 */
export async function copilotListAccounts(): Promise<GitHubAccount[]> {
  return invoke<GitHubAccount[]>("copilot_list_accounts");
}

/**
 * Poll for the OAuth token (multi-account)
 *
 * Polls GitHub with the device code until the user finishes authorizing.
 * Returns the newly added account once authorized.
 *
 * @param deviceCode - device code
 * @returns the newly added account, or null while still waiting
 */
export async function copilotPollForAccount(
  deviceCode: string,
): Promise<GitHubAccount | null> {
  return invoke<GitHubAccount | null>("copilot_poll_for_account", {
    deviceCode,
  });
}

/**
 * Remove a GitHub account
 *
 * @param accountId - GitHub user ID
 */
export async function copilotRemoveAccount(accountId: string): Promise<void> {
  return invoke("copilot_remove_account", { accountId });
}

/**
 * Set the default GitHub account
 *
 * @param accountId - GitHub user ID
 */
export async function copilotSetDefaultAccount(
  accountId: string,
): Promise<void> {
  return invoke("copilot_set_default_account", { accountId });
}

/**
 * Get a valid Copilot token for an account
 *
 * Internal, used for proxied requests.
 *
 * @param accountId - GitHub user ID
 * @returns Copilot Token
 */
export async function copilotGetTokenForAccount(
  accountId: string,
): Promise<string> {
  return invoke<string>("copilot_get_token_for_account", { accountId });
}

/**
 * Get the Copilot available models for an account
 *
 * @param accountId - GitHub user ID
 * @returns available models
 */
export async function copilotGetModelsForAccount(
  accountId: string,
): Promise<CopilotModel[]> {
  return invoke<CopilotModel[]>("copilot_get_models_for_account", {
    accountId,
  });
}

/**
 * Get Copilot usage info for an account
 *
 * @param accountId - GitHub user ID
 * @returns usage info
 */
export async function copilotGetUsageForAccount(
  accountId: string,
): Promise<CopilotUsageResponse> {
  return invoke<CopilotUsageResponse>("copilot_get_usage_for_account", {
    accountId,
  });
}
