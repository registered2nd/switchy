import { http, HttpResponse } from "msw";
import type { AppId } from "@/lib/api/types";
import type { Provider, Settings } from "@/types";
import {
  addProvider,
  deleteProvider,
  getCurrentProviderId,
  getLiveProviderIds,
  getProviders,
  listProviders,
  resetProviderState,
  setCurrentProviderId,
  updateProvider,
  updateSortOrder,
  getSettings,
  setSettings,
  getAppConfigDirOverride,
  setAppConfigDirOverrideState,
  getAccounts,
  saveAccount,
  deleteAccount,
} from "./state";
import type { Account } from "@/lib/api/accounts";
import signInTemplates from "@/config/signInTemplates.json";

const TAURI_ENDPOINT = "http://tauri.local";

const withJson = async <T>(request: Request): Promise<T> => {
  try {
    const body = await request.text();
    if (!body) return {} as T;
    return JSON.parse(body) as T;
  } catch {
    return {} as T;
  }
};

const success = <T>(payload: T) => HttpResponse.json(payload as any);

export const handlers = [
  http.post(`${TAURI_ENDPOINT}/get_accounts`, () => success(getAccounts())),
  http.post(`${TAURI_ENDPOINT}/save_account`, async ({ request }) => {
    const { account } = await withJson<{ account: Account }>(request);
    saveAccount(account);
    return success(null);
  }),
  http.post(`${TAURI_ENDPOINT}/delete_account`, async ({ request }) => {
    const { id } = await withJson<{ id: string }>(request);
    deleteAccount(id);
    return success(null);
  }),
  http.post(`${TAURI_ENDPOINT}/get_account_balance`, () =>
    success({
      success: true,
      error: null,
      currency: "USD",
      balance: 12.5,
      parts: [{ name: "toppedUp", amount: 12.5 }],
      spentToday: 0.5,
      spentMonth: 3,
      trackingSince: Date.now() - 86_400_000,
      queriedAt: Date.now(),
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/use_account_in`, async ({ request }) => {
    const { id, app } = await withJson<{ id: string; app: AppId }>(request);
    const account = getAccounts().find((a) => a.id === id);
    if (!account)
      return HttpResponse.json("Account not found", { status: 400 });
    const existing = Object.values(listProviders(app)).find(
      (p) => p.meta?.accountId === id,
    );
    const providerId = existing?.id ?? `${account.company}-${id}`;
    if (!existing) {
      addProvider(app, {
        id: providerId,
        name: `${account.company} · ${account.name}`,
        settingsConfig: { env: { ANTHROPIC_AUTH_TOKEN: account.secret } },
        meta: { accountId: id },
      } as Provider);
    }
    setCurrentProviderId(app, providerId);
    return success(providerId);
  }),
  http.post(`${TAURI_ENDPOINT}/add_sign_in_account`, async ({ request }) => {
    const { app, name } = await withJson<{ app: AppId; name?: string }>(
      request,
    );
    const template = (signInTemplates as Record<string, any>)[app];
    if (!template) return HttpResponse.json("No sign-in", { status: 400 });
    const id = `signin-${Date.now()}`;
    addProvider(app, {
      id,
      name: name || template.name,
      settingsConfig: template.settingsConfig,
      category: "official",
    } as Provider);
    return success(id);
  }),
  http.post(`${TAURI_ENDPOINT}/remove_account_from`, () => success(null)),
  http.post(`${TAURI_ENDPOINT}/get_account_quota`, () =>
    success({
      tool: "coding_plan",
      credentialStatus: "valid",
      credentialMessage: null,
      success: true,
      tiers: [{ name: "five_hour", utilization: 10, resetsAt: null }],
      extraUsage: null,
      error: null,
      queriedAt: Date.now(),
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/get_coding_plan_quota`, () =>
    success({
      tool: "coding_plan",
      credentialStatus: "valid",
      credentialMessage: null,
      success: true,
      tiers: [{ name: "five_hour", utilization: 10, resetsAt: null }],
      extraUsage: null,
      error: null,
      queriedAt: Date.now(),
    }),
  ),

  http.post(`${TAURI_ENDPOINT}/get_providers`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(getProviders(app));
  }),

  http.post(`${TAURI_ENDPOINT}/get_current_provider`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(getCurrentProviderId(app));
  }),

  http.post(
    `${TAURI_ENDPOINT}/update_providers_sort_order`,
    async ({ request }) => {
      const { updates = [], app } = await withJson<{
        updates: { id: string; sortIndex: number }[];
        app: AppId;
      }>(request);
      updateSortOrder(app, updates);
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/update_tray_menu`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_opencode_live_provider_ids`, () =>
    success(getLiveProviderIds("opencode")),
  ),

  http.post(`${TAURI_ENDPOINT}/get_openclaw_live_provider_ids`, () =>
    success(getLiveProviderIds("openclaw")),
  ),

  http.post(`${TAURI_ENDPOINT}/get_openclaw_default_model`, () =>
    success({ primary: null, fallback: [] }),
  ),

  http.post(`${TAURI_ENDPOINT}/scan_openclaw_config_health`, () => success([])),

  http.post(`${TAURI_ENDPOINT}/switch_provider`, async ({ request }) => {
    const { id, app } = await withJson<{ id: string; app: AppId }>(request);
    const providers = listProviders(app);
    if (!providers[id]) {
      return HttpResponse.json(false, { status: 404 });
    }
    setCurrentProviderId(app, id);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/add_provider`, async ({ request }) => {
    const { provider, app } = await withJson<{
      provider: Provider & { id?: string };
      app: AppId;
    }>(request);

    const newId = provider.id ?? `mock-${Date.now()}`;
    addProvider(app, { ...provider, id: newId });
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/update_provider`, async ({ request }) => {
    const { provider, app } = await withJson<{
      provider: Provider;
      app: AppId;
    }>(request);
    updateProvider(app, provider);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/delete_provider`, async ({ request }) => {
    const { id, app } = await withJson<{ id: string; app: AppId }>(request);
    deleteProvider(app, id);
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/import_default_config`, async () => {
    resetProviderState();
    return success(true);
  }),

  http.post(`${TAURI_ENDPOINT}/open_external`, () => success(true)),

  // MCP APIs

  http.post(`${TAURI_ENDPOINT}/restart_app`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_settings`, () => success(getSettings())),

  http.post(`${TAURI_ENDPOINT}/check_env_conflicts`, () => success([])),

  http.post(`${TAURI_ENDPOINT}/save_settings`, async ({ request }) => {
    const { settings } = await withJson<{ settings: Settings }>(request);
    setSettings(settings);
    return success(true);
  }),

  http.post(
    `${TAURI_ENDPOINT}/set_app_config_dir_override`,
    async ({ request }) => {
      const { path } = await withJson<{ path: string | null }>(request);
      setAppConfigDirOverrideState(path ?? null);
      return success(true);
    },
  ),

  http.post(`${TAURI_ENDPOINT}/get_app_config_dir_override`, () =>
    success(getAppConfigDirOverride()),
  ),

  http.post(`${TAURI_ENDPOINT}/get_default_claude_mirror_dir`, () =>
    success(null),
  ),

  http.post(`${TAURI_ENDPOINT}/get_default_codex_mirror_dir`, () =>
    success(null),
  ),

  http.post(`${TAURI_ENDPOINT}/get_config_dir`, async ({ request }) => {
    const { app } = await withJson<{ app: AppId }>(request);
    return success(app === "claude" ? "/default/claude" : "/default/codex");
  }),

  http.post(
    `${TAURI_ENDPOINT}/select_config_directory`,
    async ({ request }) => {
      const { defaultPath, default_path } = await withJson<{
        defaultPath?: string;
        default_path?: string;
      }>(request);
      const initial = defaultPath ?? default_path;
      return success(initial ? `${initial}/picked` : "/mock/selected-dir");
    },
  ),

  http.post(`${TAURI_ENDPOINT}/pick_directory`, async ({ request }) => {
    const { defaultPath, default_path } = await withJson<{
      defaultPath?: string;
      default_path?: string;
    }>(request);
    const initial = defaultPath ?? default_path;
    return success(initial ? `${initial}/picked` : "/mock/selected-dir");
  }),

  http.post(`${TAURI_ENDPOINT}/open_file_dialog`, () =>
    success("/mock/import-settings.json"),
  ),

  http.post(
    `${TAURI_ENDPOINT}/import_config_from_file`,
    async ({ request }) => {
      const { filePath } = await withJson<{ filePath: string }>(request);
      if (!filePath) {
        return success({ success: false, message: "Missing file" });
      }
      setSettings({ language: "en" });
      return success({ success: true, backupId: "backup-123" });
    },
  ),

  http.post(`${TAURI_ENDPOINT}/export_config_to_file`, async ({ request }) => {
    const { filePath } = await withJson<{ filePath: string }>(request);
    if (!filePath) {
      return success({ success: false, message: "Invalid destination" });
    }
    return success({ success: true, filePath });
  }),

  http.post(`${TAURI_ENDPOINT}/save_file_dialog`, () =>
    success("/mock/export-settings.json"),
  ),

  // Sync current providers live (no-op success)
  http.post(`${TAURI_ENDPOINT}/sync_current_providers_live`, () =>
    success({ success: true }),
  ),

  // Proxy status (for SettingsPage / ProxyPanel hooks)
  http.post(`${TAURI_ENDPOINT}/get_proxy_status`, () =>
    success({
      running: false,
      address: "127.0.0.1",
      port: 0,
      active_connections: 0,
      total_requests: 0,
      success_requests: 0,
      failed_requests: 0,
      success_rate: 0,
      uptime_seconds: 0,
      current_provider: null,
      current_provider_id: null,
      last_request_at: null,
      last_error: null,
      failover_count: 0,
      active_targets: [],
    }),
  ),

  http.post(`${TAURI_ENDPOINT}/get_proxy_takeover_status`, () =>
    success({
      claude: false,
      codex: false,
      gemini: false,
    }),
  ),

  http.post(`${TAURI_ENDPOINT}/is_live_takeover_active`, () => success(false)),

  // Failover / circuit breaker defaults
  http.post(`${TAURI_ENDPOINT}/get_failover_queue`, () => success([])),
  http.post(`${TAURI_ENDPOINT}/get_available_providers_for_failover`, () =>
    success([]),
  ),
  http.post(`${TAURI_ENDPOINT}/add_to_failover_queue`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/remove_from_failover_queue`, () =>
    success(true),
  ),
  http.post(`${TAURI_ENDPOINT}/reorder_failover_queue`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/set_failover_item_enabled`, () => success(true)),

  http.post(`${TAURI_ENDPOINT}/get_circuit_breaker_config`, () =>
    success({
      failureThreshold: 3,
      successThreshold: 2,
      timeoutSeconds: 60,
      errorRateThreshold: 50,
      minRequests: 5,
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/update_circuit_breaker_config`, () =>
    success(true),
  ),
  http.post(`${TAURI_ENDPOINT}/get_provider_health`, () =>
    success({
      provider_id: "mock-provider",
      app_type: "claude",
      is_healthy: true,
      consecutive_failures: 0,
      last_success_at: null,
      last_failure_at: null,
      last_error: null,
      updated_at: new Date().toISOString(),
    }),
  ),
  http.post(`${TAURI_ENDPOINT}/reset_circuit_breaker`, () => success(true)),
  http.post(`${TAURI_ENDPOINT}/get_circuit_breaker_stats`, () => success(null)),
];
